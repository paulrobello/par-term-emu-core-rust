# par-mux — Terminal Multiplexer Daemon

`par-mux` is a tmux-control-mode multiplexer daemon: it owns PTY-backed panes arranged in a workspace/session/window/pane tree and serves the tmux control-mode protocol over a local socket, so control-mode clients (including `par-term-tmux` and the `MuxClient` in this crate) can attach to it in place of tmux. It also carries an agent layer: panes can report agent state over the same socket, and agent sessions survive daemon restarts.

The daemon is feature-gated (Rust `mux` feature for the library, `mux-bin` for the `par-mux` binary), optional, and independent of the Python bindings and the streaming server. The wire format's conformance oracle is the `TmuxControlParser` in `crates/par-term-emu-core/src/tmux_control/` — the daemon emits what that parser decodes.

> **Design decisions:** code comments cite D-numbered decisions (`D3.3`, `D5`, …) summarized one line each in [MUX_DECISIONS.md](MUX_DECISIONS.md); the full plan lives in the `par-agent-os` repository (see [par-mux.md](par-mux.md)).

## Binary Updates

`par-mux --check-update` reports the latest stable release without changing the installation.
`par-mux --self-update` installs it and starts the new executable with
`--socket <same socket> --restart --no-attach`. The existing stop path saves final state, then
fresh startup restores sessions, screens, working directories, and agent resume identities.
Pane processes respawn, rather than transferring live PTYs. Named and explicit sockets retain their target.
The existing `par-mux update [--check] [--channel stable|preview]` uses the same checksum-required engine.

Automatic installation is opt-in in the canonical config (`dirs::config_dir()/par-mux/config.toml`,
`PAR_MUX_CONFIG` overrides it). These settings are read at daemon startup, not live-reloaded:

```toml
[daemon]
auto-update = false
update-interval = "daily" # hourly | daily | weekly
```

When enabled, a background worker checks at startup and at the configured interval while serving.
The default interval is daily, including when the key is omitted. Unknown intervals are rejected.
Updates require HTTPS access, `curl` and `tar` on PATH, and directory write access. Exact platform
archives and `par-mux-v<version>-SHA256SUMS.txt` are fetched from this repository's GitHub release.
A missing sidecar or digest mismatch refuses the update before touching the installed binary.
A successful replacement leaves `<executable>.previous` beside it.

An executable under `*.app/Contents/` is refused because replacement can invalidate its signature
and TCC grant. Only the explicit `--self-update --force-self-update` bypasses this guard. Automatic
updates never force a bundle replacement. Use the bundle's normal installation procedure instead.

## Table of Contents

- [Building and Running](#building-and-running)
- [Command Line](#command-line)
- [Client Mode](#client-mode)
- [Discovering servers](#discovering-servers)
- [Attaching from a terminal](#attaching-from-a-terminal)
- [Socket and State Paths](#socket-and-state-paths)
- [Protocol Overview](#protocol-overview)
- [Command Reference](#command-reference)
- [Git worktree workspaces](#git-worktree-workspaces)
- [Raw pane streams](#raw-pane-streams)
- [Notifications](#notifications)
- [Agent Hook Reports](#agent-hook-reports)
- [Host Telemetry Probe](#host-telemetry-probe)
- [Agent Scrape Tier](#agent-scrape-tier)
- [Self-Update and Installers](#self-update-and-installers)
- [Plugin System](#plugin-system)
- [Saved Machines and the SSH Thin Client](#saved-machines-and-the-ssh-thin-client)
- [Persistence and Restart](#persistence-and-restart)
- [Pane Reaping](#pane-reaping)
- [Shutdown Semantics](#shutdown-semantics)
- [Streaming Panes to the Web](#streaming-panes-to-the-web)
- [Embedding from Rust](#embedding-from-rust)
- [macOS App Bundle (TCC Permissions and Autostart)](#macos-app-bundle-tcc-permissions-and-autostart)
- [Testing](#testing)
- [Module Map](#module-map)

## Building and Running

The daemon binary is not part of the default build. Build it with the `mux-bin` feature (`mux` plus the `clap` CLI parser):

```bash
cargo build -p par-mux --bin par-mux --features mux-bin
```

Run it (it prints its socket path and serves until killed — or until it empties: no sessions, or only dead panes, for 5 s; attached clients do not hold it, see [Shutdown Semantics](#shutdown-semantics)):

```bash
cargo run -p par-mux --bin par-mux --features mux-bin
```

The Python wheel and the default `make dev` build do not include the daemon.

`make completions` regenerates the tracked shell-completion scripts under `crates/par-mux/completions/` from the `mux-bin,attach` build, and `make completions-check` (part of `checkall`) fails when they drift from the CLI — run `make completions` and commit the result after changing any flag, subcommand, or flag doc comment. See [Shell completions](#shell-completions).

### Standalone binaries (GitHub Releases)

Standalone `par-mux` executables — no Python and no par-term installation
required, built from the `rust-only,mux-bin,attach` feature set (daemon, CLI
client, and the `attach` TUI) — are attached to every
[GitHub Release](https://github.com/paulrobello/par-term-emu-core-rust/releases)
from the release that introduces them on. Each release carries one archive per platform
(`par-mux-v<version>-<target>.tar.gz` on Unix, `.zip` on Windows) plus a
`par-mux-v<version>-SHA256SUMS.txt` covering all of them:

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `par-mux-v<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `par-mux-v<version>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `par-mux-v<version>-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `par-mux-v<version>-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `par-mux-v<version>-x86_64-pc-windows-msvc.zip` |

Every archive also ships the five shell-completion scripts (bash, zsh, fish, PowerShell, elvish) under `completions/` — or print one any time with `par-mux completion <shell>`:

```bash
par-mux completion bash > ~/.local/share/bash-completion/completions/par-mux
par-mux completion zsh   > ~/.zfunc/_par-mux        # a directory on $fpath
par-mux completion fish  > ~/.config/fish/completions/par-mux.fish
```

Install on Linux/macOS (pick the archive for your target; each archive
extracts to a `par-mux-v<version>-<target>/` directory containing the
executable, `LICENSE`, and an install note):

```bash
curl -LO https://github.com/paulrobello/par-term-emu-core-rust/releases/latest/download/par-mux-v<version>-x86_64-unknown-linux-gnu.tar.gz
tar -xzf par-mux-v<version>-x86_64-unknown-linux-gnu.tar.gz
install -m 755 par-mux-v<version>-x86_64-unknown-linux-gnu/par-mux ~/.local/bin/par-mux
```

Ensure `~/.local/bin` is on `PATH`. On Windows, extract the `.zip`, create a
directory for the executable (e.g. `%LOCALAPPDATA%\Programs\par-mux`), add
that directory to `PATH`, and copy `par-mux.exe` into it. Verify any download
against the checksum file from the same release with
`sha256sum -c par-mux-v<version>-SHA256SUMS.txt` (Windows:
`certutil -hashfile <archive> SHA256`).

The same feature set builds from source (this is exactly what the release
binaries are built with):

```bash
cargo install --path crates/par-mux --locked --features mux-bin,attach --bin par-mux
```

or from crates.io. Since 0.58.1 the daemon and the attach client ship as
their own `par-mux` crate:

```bash
cargo install par-mux --features mux-bin,attach
```

At startup the daemon raises its `RLIMIT_NOFILE` soft limit toward the hard limit (Unix), logging the old and new values — an inherited launchd-style limit of 256 descriptors caps the daemon near 60 panes (~4 descriptors each), which the raise removes. An unbounded hard limit is treated as 8192.

## Command Line

The daemon parses its arguments with `clap` (`--help` and `--version` both work):

```text
par-mux <name>              Start (or restart into) the default-socket daemon for <name> and attach
par-mux --socket <path>     The same, on an explicit socket path
par-mux                     Same as par-mux default
par-mux --state-dir <dir>   Override the platform state directory the tree is persisted under
par-mux [<name>] --foreground        Serve in the foreground of this process — no daemonize, no attach
par-mux [<name>] --no-attach         Daemonize as usual but exit daemon-only, without attaching
par-mux [<name>] --stop     Stop the daemon on this socket cleanly and wait for it to exit
par-mux [<name>] --restart  Stop, then start a fresh daemon on the same socket (detached, attach)
par-mux [<name>] --cmd CMD  Send one control command to the running daemon and print the reply
par-mux --pane-endpoints          Give each pane its own hook-only socket (see Agent Hook Reports)
par-mux --expose-control-socket   With --pane-endpoints: also export PAR_MUX_CONTROL_SOCKET in panes
par-mux --gen-config [--force]    Write the config file with the current effective settings
par-mux --install-skill [--skills-dir DIR]  Install the bundled agent skill into the Claude Code skills directory
par-mux --print-schema            Print the bundled JSON command schema and exit (see list-commands -J)
par-mux --print-completions SHELL Print the shell-completion script (bash, zsh, fish, powershell, elvish) and exit
```

The `[daemon]` section of `<config dir>/par-mux/config.toml` carries
defaults for the socket, state dir, and the two endpoint switches — see
[Configuration file](#configuration-file).

`--socket <path>` is what `MuxClient::connect_or_spawn_at` passes when it starts a daemon. A second daemon on a path a live server already owns is refused with "another server owns <path>"; a stale socket remnant (dead socket file, Windows marker file, or a stray regular file at the path) is reclaimed.

`--stop` and `--restart` are flags rather than subcommands — the positional `NAME` would otherwise be ambiguous with a session literally named `stop`.
`--stop` sends `kill-server` to the daemon on that socket and waits (30 s bound) for the socket to stop accepting connections; "no daemon running" is reported but is not an error.
The daemon unlinks the socket only AFTER its final state save is on disk, so the wait's return is also the guarantee that the save has landed — a restart racing its predecessor's save read a missing or stale state file before this ordering held. Note the reply-before-exit window: the `kill-server` reply lands before the daemon's final save and exit, so a script's `--stop; rm -rf state` can delete the dir inside it; the final save respects that — a shutdown-family save whose target dir is gone skips instead of re-creating it (pinned by `a_shutdown_save_does_not_resurrect_a_deleted_state_dir`).
`--restart` does the same stop, then brings the same socket back as a fresh daemon — the state save the stop just completed is what it restores.
Before the fork, the invocation reports on the terminal what the fresh daemon will restore: nothing (with the reason — no saved state, or the previous daemon saved an empty tree) when the restore will be empty, and where the details live.
The fresh daemon's stderr is routed to `<state file>.log` beside the state file (created `0600`, appended across restarts) instead of discarded, so a startup failure — a bind error, a restore failure, an exit-when-empty — leaves evidence there after the terminal is gone.
The daemon detaches by default (fork + `setsid`; stdin/stdout to `/dev/null`), so no `&` is needed and closing the terminal it was typed into leaves the daemon and its panes running — herdr's launch shape: `par-mux <name>` starts the daemon and ATTACHES the invoking terminal to it, and `--restart` lands in the fresh daemon the same way. The launch process waits (10 s bound) for the socket to accept, then either execs the attach client in place (`par-mux attach --socket …`) or exits: `--no-attach` skips the attach on request, and a start whose stdin is not a terminal (a script, a boot job) stays daemon-only by itself. A socket that never accepts is an error naming the daemon log, because after the fork the child's failures are only visible there. `--foreground` is the escape: this exact process serves, no fork and no attach — the debugging and supervision shape, and the one every programmatic spawner uses (`MuxClient`'s auto-spawn and the test harness pass it, because they kill the child they spawned and a detached grandchild could never be named). On Windows the daemon always serves in the foreground of the invoking process (the fork shape is unix); `--foreground` and `--no-attach` parse there but have no one to attach. `--restart` composes with both modifiers.
This is the routine fix after rebuilding par-mux, since clients attach to whatever daemon owns the socket and an old daemon keeps serving old code until restarted.

`--pane-endpoints` (opt-in, default off) gives every pane its own socket endpoint that accepts only hook reports — see [Agent Hook Reports](#agent-hook-reports). `--expose-control-socket` accompanies it: panes then also get `PAR_MUX_CONTROL_SOCKET` naming the full control socket, and a session env of `PAR_MUX_CONTROL=1` grants the same per session.

`--install-skill` writes the agent skill embedded in the binary (the
`skills/par-mux/SKILL.md` this repository ships — the docs a coding agent
loads to drive this CLI) to `~/.claude/skills/par-mux/SKILL.md`, creating
the directory, and prints the installed path. Always overwrites: the
embedded copy is the source of truth, so a re-install after an upgrade
refreshes the skill; `--skills-dir DIR` retargets the skills root (tests,
other agent harnesses). Like `--cmd`, it never starts or contacts a daemon,
so it works with nothing running and from inside a pane. par-term invokes
it after installing par-mux instead of embedding its own copy of the skill,
so what an agent reads always matches the binary it drives.

### Shell completions

`--print-completions SHELL` writes a completion script for `bash`, `zsh`,
`fish`, `powershell`, or `elvish` to stdout and exits; like `--print-schema`
it never contacts a daemon. The scripts are generated from the clap
definition itself, so they cover every flag and subcommand the binary
parses. Generated copies are also tracked under `crates/par-mux/completions/`
(`par-mux.bash`, `_par-mux`, `par-mux.fish`, `_par-mux.ps1`, `par-mux.elv`).
par-mux never edits a shell rc file; install one by hand:

```bash
par-mux --print-completions bash > ~/.local/share/bash-completion/completions/par-mux
par-mux --print-completions zsh > ~/.zfunc/_par-mux        # a directory on $fpath, before compinit
par-mux --print-completions fish > ~/.config/fish/completions/par-mux.fish
par-mux --print-completions powershell >> $PROFILE           # or dot-source a saved copy
par-mux --print-completions elvish > ~/.config/elvish/lib/par-mux.elv   # then: use par-mux
```

### Nested daemons

Serve mode refuses to start inside a par-mux pane: when `PAR_MUX_ENV` is set (the pane env contract marks every pane), `par-mux <name>` / `par-mux --socket <path>` exits non-zero with `refusing to start a nested daemon: PAR_MUX_ENV is set … set PAR_MUX_ALLOW_NESTED=1 to override` and binds no socket — checked in the launching process, before the default fork, so the refusal lands on the terminal rather than in the daemon log — a nested daemon would shadow the outer server's identity for every PTY under it. `MuxClient::connect_or_spawn` applies the same rule: from inside a pane it attaches to a daemon that is already running but refuses to auto-spawn one, failing fast instead of retrying a socket the guard keeps unbound.

Exempt from the guard, because a pane must keep operating on its own daemon: `--cmd` (client mode never starts a daemon), `--stop`, and `--restart` — tmux likewise allows `kill-server` from inside a session. `PAR_MUX_ALLOW_NESTED=1` (or unsetting `PAR_MUX_ENV`) starts a nested daemon anyway.

Test harnesses that own a daemon inside a pane use the override deliberately: set `PAR_MUX_ALLOW_NESTED=1` for the daemon invocation (or unset `PAR_MUX_ENV` in its environment) and give the nested daemon an explicit private `--socket` and `--state-dir` of its own, e.g. `PAR_MUX_ALLOW_NESTED=1 par-mux --socket /tmp/harness.sock --state-dir /tmp/harness-state --foreground` — so the harness's daemon never lands on the pane's own socket or state. The guard's default refusal exists so an accidental serve mode in a pane cannot shadow the outer server's identity for every PTY under it; the override is the sanctioned escape for a harness that owns its daemon and its paths, not a variable to set globally.

Panes never inherit a stale outer identity: a PTY spawned by any process inside a mux pane drops every `PAR_MUX_*` variable from its inherited environment and re-adds only its own via the env contract below — so a par-term started in a pane gets clean local tabs instead of reporting its agents to the outer daemon under the wrong pane id. The same drop removes the outer agent session's identity vars (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_MESSAGING_TOKEN`, `OMPCODE`, `CODEX_THREAD_ID` — herdr parity): a pane is not a child agent of whatever started the daemon, and nested-session detection keyed on these vars (omp treats `OMPCODE=1` as nested and never reports) would hide the pane's own agents from rosters. `set-environment`/`new-session -e` (or any explicit `set_env`) opts back in for an intentional child session.

## Client Mode

`par-mux --cmd '<command>'` (short form `-c`) connects to the daemon that owns the socket, sends one control command, prints the reply, and exits. It drives panes from a shell or script without linking `MuxClient`. The target socket resolves exactly as the daemon's does: the positional `<name>` (default `default`) maps to the named default path, and `--socket <path>` overrides it. Like `--stop`, it is a flag rather than a subcommand so the positional name stays unambiguous.

```bash
# Type a command into pane %3 and press Enter (send-keys appends nothing; Enter is a key)
par-mux --cmd 'send-keys -t %3 "ls -la" Enter'

# Capture a pane's visible screen, or 200 lines of history plus the screen
par-mux --cmd 'capture-pane -t %3'
par-mux --cmd 'capture-pane -t %3 -S -200'

# Block until pane %3 prints a line matching the regex (exit 0, the line on stdout),
# or give up after 30 s (exit 1, the timeout on stderr)
par-mux --cmd "wait-output -t %3 -r 'tests? (passed|failed)' -T 30000"

# Launch claude into pane %3 (at its shell prompt) and return once it is detected,
# prompt it and block until the turn ends, and ask how its state is detected
par-mux --cmd 'agent-start -t %3 claude'
par-mux --cmd 'agent-prompt -t %3 -w -T 600000 -- fix the failing test'
par-mux --cmd 'agent-explain -t %3'

# The agent roster, and a split of pane %3
par-mux --cmd list-agents
par-mux --cmd 'split-window -h -t %3'

# A named daemon, an explicit socket, or (inside a pane) the socket the pane spawned with
par-mux work --cmd list-panes
par-mux --socket /tmp/par-mux-test.sock --cmd version
par-mux --cmd list-sessions   # resolves $PAR_MUX_SOCKET before the default socket

# The one-shot type-and-run form (R5): --enter appends the Enter byte
par-mux --cmd "send-keys -t %3 -l 'make check' --enter"

# Existence probe without roster parsing; exit 0/1
par-mux --cmd 'has-session -t work'

# Stable field output for scripts
par-mux --cmd "list-sessions -F '#{session_id}=#{session_name}'"

# Live daemon settings
par-mux --cmd show-options
par-mux --cmd 'set-option exit-empty off'
```

| Outcome | stdout | stderr | Exit code |
|---------|--------|--------|-----------|
| Success (`%end`) | The reply body, one line per reply line, framing stripped; nothing for an empty reply | — | 0 |
| Command failed (`%error`) | — | The daemon's error text | 1 |
| No daemon owns the socket | — | `no daemon running on <path>` | 1 |
| Transport failure after connecting (no reply block, connection closed) | — | The I/O error | 2 |
| Target is a pane endpoint (`--pane-endpoints` daemon) | — | `this pane has hook-only access; start the daemon with --expose-control-socket or pass --socket` | 1 |

Client mode never starts a daemon: a bare `par-mux --cmd list-sessions` with nothing running fails immediately instead of booting one (the error carries the start hint). The command string follows the daemon's grammar (see the quoting rules under [Command Reference](#command-reference)); wrap it in single quotes in the shell so its inner double quotes reach the daemon intact. One command per invocation — there is no `;` sequencing, interactive attach, or follow mode. Pushed notifications that arrive while the reply is pending are discarded. Stdout writes stop quietly on a closed pipe, so `par-mux --cmd '...' | head -1` does not panic.

### Subcommands

The verb forms are the discoverable front door (every long-standing flag keeps working — `par-mux --stop` and `par-mux stop` are the same handler):

| Subcommand | Same as | Notes |
|------------|---------|-------|
| `par-mux start [NAME] [--socket P] [--state-dir D] [--foreground] [--no-attach] [--pane-endpoints] [--expose-control-socket]` | bare `par-mux [NAME]` | The daemon-start action, spelled as a verb |
| `par-mux stop [NAME\|--socket P]` | `--stop` | Clean shutdown, waits for the socket |
| `par-mux restart [NAME\|--socket P]` | `--restart` | Stop + fresh start restoring the saved tree |
| `par-mux status [NAME\|--socket P]` | — | The orientation block: `par-mux <socket> <stamp>`, `workspaces:`, `sessions:`, `displayed: $N`, `panes: N` — the orient step in one invocation; each section degrades to `?` rather than failing the reply |
| `par-mux ls [NAME\|--socket P]` | `--cmd list-sessions` | The tmux-habit session listing |
| `par-mux attach …` | the attach client | Feature `attach`; see [Attaching from a terminal](#attaching-from-a-terminal) |
| `par-mux config [--force]` | `--gen-config [--force]` | Write the effective config file |
| `par-mux observe -t PANE [--no-seed] [NAME\|--socket P]` | — | Stream one pane's screen and live output to stdout, read-only; see [Raw pane streams](#raw-pane-streams) |
| `par-mux control -t PANE [--no-seed] [--size WxH] [NAME\|--socket P]` | — | `observe` plus stdin forwarded to the pane and this terminal's size reported; see [Raw pane streams](#raw-pane-streams) |

A subcommand shadows the same-named NAME-form daemon start (`par-mux attach` never starts a daemon named "attach" — that needs `--socket`). The bare NAME form carries a collision guard of its own: a name that spells a control command (`par-mux list-sessions`) is refused with a "command word" hint instead of silently starting a stray daemon; an explicit `--socket` alongside the name reads as deliberate and is allowed. The daemon name `"server"` is reserved outright (Paul, 2026-10-18): `par-mux server stop` reads as "stop the server" but addresses a daemon literally named "server" (socket `par-mux-server.sock`), so every name position — bare, verb-first (`par-mux server stop`), or verb-last (`par-mux start server`, `status server`) — refuses it with the default-daemon spelling in the explanation; only `--socket` reads as deliberate (pinned by `the_reserved_name_server_is_refused_in_every_name_position`).

### Discovering servers

`par-mux --list-servers` enumerates every par-mux daemon this user can reach and prints one line per LIVE server — the label, socket path, the daemon's pid and age from its registry entry, session count, build stamp, a `[default]` marker on the socket default resolution picks, and a `(pid-named)` marker on generated `$NN` names (a leaked test daemon is recognizable at a glance) — so `attach` has something to enumerate:

```bash
par-mux --list-servers
# work  /…/par-mux-work.sock  pid=50192 up=3h40m  sessions=2  0.62.0+a1b2c3d
# $10  /…/par-mux-$10.sock  pid=?  sessions=1  0.62.0+d4e5f6a  (pid-named)
```

The multi-server attach refusal prints the same rows (marking the default) and names the composed form: `par-mux attach <name|path>` with `-t <session>` appended when the server holds several sessions; the multi-session refusal names the server-qualified spelling too, so a suggested form never fails against a second running server.

Each daemon writes one pointer file into its state directory at bind (`<state-base>/servers/<socket-stem>.json`: socket path, pid, start time, build stamp) and removes it on the clean shutdown path (`kill-server`/`--stop`). A crash leaves the file behind; the next list-servers run probes every registered socket with `version` + `list-sessions` through the same transport a client uses and prunes the entries whose probe fails silently (the enumeration owns cleanup; the per-entry "pruned" notes were noise in every caller's output), and — when the registry base is the platform default — also sweeps the named-default socket directory, so named daemons an older binary never registered still appear. `--list-servers` honors `--state-dir`, which scopes both the registry and the sweep to that base (what tests and sandboxes use). Registry failure never blocks a daemon from serving; discovery then simply falls back to the directory sweep.

`par-mux attach` with no NAME and no `--socket` while MORE THAN ONE server is live refuses with the same list plus a one-line hint (`attach to one with par-mux attach <name|path>`, exit 1) instead of silently picking one; exactly one live server attaches to it directly.

## Attaching from a terminal

`par-mux attach` (feature `attach`; `par-mux attach [-t TARGET] [--prefix KEY] [--mode render|passthrough] [NAME | --socket PATH]`) is the built-in client.
With no `--mode` and no `[client] mode` in the config file it runs the render pipeline (the full TUI: every pane of the window, tab strip, status bar — see Render mode below); `--mode passthrough` (or `mode = "passthrough"` in the config file) selects the byte pump described next: one pane fullscreen in your terminal, the host terminal acting as the pane's VT emulator — a byte pump plus key pump plus a one-row status line, tmux's attach model minus the window chrome.
`--mode` beats the config file, which beats the render default.
Passthrough grants pane programs the host terminal's full escape-sequence capability (clipboard writes, hyperlinks, query replies), so prefer render mode for untrusted output; see [SECURITY.md — Attach Modes](SECURITY.md#attach-modes).
Build with `--features mux-bin,attach`.

```bash
par-mux attach                      # render mode, the displayed session's active window
par-mux attach -t %3                # that pane
par-mux attach -t @1                # that window's active pane
par-mux attach -t work              # pane title / window name / session name, daemon-resolved
par-mux attach work --prefix C-a    # named daemon, custom prefix
```

### Startup (both modes)

The handshake runs in the client-contract order (`version`, `list-commands`, `set-client-colors`, the `refresh-client -C WxH -p WxH` size report), the target resolves to a pane (`%N` directly; a window/session target narrows through the targeted `list-panes`/`list-windows` queries to the marked pane; no `-t` takes the highest pane id (ids are monotonic, so the max is the newest pane)), and the pane's screen-restore replay goes to your terminal verbatim.
The daemon then pushes `%output` for that pane as it arrives, octal-decoded to raw bytes — mouse tracking, bracketed paste, and every graphics protocol work for free in passthrough: the pane's escape bytes flow to the host emulator, and the host's reports flow back on stdin.

### Passthrough mode

#### Passthrough keys

Stdin bytes are forwarded to the pane in chunked `send-keys -H` (~512 bytes per command). The prefix (default `C-b`, tmux's; `[client] prefix` in the config file) is intercepted: `d` detaches; `o` and the arrow keys cycle panes; `n`/`p` move to the next/previous window; `(`/`)` move to the previous/next session; `W`/`C-w` select the next/previous workspace (`select-workspace -t +N` in id order — the client lands on the workspace's session's active window through the same resync,
and a workspace with no sessions moves the selection without a landing; prefix `w` opens the session/window picker modal — sessions with their windows nested, the current one emphasized, keyboard and mouse navigable (`[client] picker`); `%` splits the focused pane right and `"` splits it down (`split-window -t <focused> [-h]` — the client lands on the fresh split through the same resync every switch uses);
`x` kills the focused pane (`kill-pane -t <focused>` — the client follows the window's surviving pane, and killing the last pane shows the exit cue instead); `!` breaks the focused pane out to a new window and lands on it (`break-pane -s <focused>`); `c` opens a new window in the focused pane's session and lands on it; `{`/`}` swap the focused pane with its layout-order neighbor (`swap-pane -s <focused> -t <neighbor>`); `R` enters a sticky resize mode — arrows move the focused pane's edges (`resize-pane -t <focused> -L|-R|-U|-D <step>`,
the wire's relative form) until Enter, Escape, or `q` exits, and any other key leaves the mode and is reprocessed normally; `?` prints the bindings panel as plain text (every chord at its effective binding — see the render-mode help bullet); `r` (`[client] respawn`) respawns the pane — but only when it is held dead (a live pane restart from a mistyped chord would be destructive,
and the daemon refuses `respawn-pane` of a running pane anyway). `C-b C-b` sends a literal `C-b` to the pane. While the prefix is armed — sent, awaiting its second key — the status line says ` prefix ` (both modes; the cue clears on the next key). The reload chord (default `C-b C-r`, `[client] reload`) re-reads the config file mid-session: prefix and reload rebind live, `config reloaded` (or the error) flashes on the status row, and `reload-config` goes to the daemon — see [Configuration file](#configuration-file). When the focused pane is held dead,
plain stdin bytes are dropped instead of forwarded — the daemon answers every `send-keys` to a dead pane with an error and the bytes would only echo into the pane's frozen screen — so the client stays quiet and the status cue explains why; prefix chords keep working. Every switch is daemon-side `select-*` followed by a `refresh-client` resync, so the redraw always shows the daemon's authoritative screen. The management chords (`%`,
`"`, `x`, `c`, `W`, `C-w`) are settable per chord in the config file (see [Configuration file](#configuration-file)).

#### Passthrough status line

The bottom terminal row is reserved with DECSTBM (the scroll region shrinks to rows 1..N-1, so pane output scrolls above it), showing the workspace names in id order (the DISPLAYED session's workspace bracketed — the daemon's active one when the session carries none), session name, pane title, the pane's agent count, and, over a held-dead pane, an `(exited N — C-b r respawns)` cue naming the restart chord (spelled from the live prefix and `[client] respawn`).
The row's facts re-query the same way the render bar does: against a daemon advertising `client-snapshot v1` one `client-snapshot -t <window>` round trip serves a refresh, and any refusal — no feature, an unknown version, an error for the tracked window — runs the legacy per-fact round (`pane-info`, `list-sessions`/`list-windows` owner scan, `list-workspaces`, `pane-title`, `list-agents`), the same spelling that discovers the tracked window in the first place.
The view does NOT use the host's alternate screen — a pane app's own `1049h`/`1049l` pair (htop, vim) would pop the client's and strand every later draw on the host main — attach and detach each clear the screen (`ESC[2J ESC[H`) instead, and every pane-show replay re-clears so switched panes never mix, keeping the client's draws on the screen it cleared.
The draw also keeps a shadow emulator of the pane (the same core `Terminal` the render mode feeds, seeded by the replay and every `%output` chunk, sized to the pane's grid, which is the host grid minus the status row, the same size the client reports), and every draw closes by placing the cursor ABSOLUTELY at the emulator's tracked cell (`ESC[<row>;<col>H`) instead of ending on the save/restore wrap alone — pane output flowing while a draw runs once raced the restore (a scroll landing between save and restore put the cursor one line off, so later output painted over the wrong row), and a fresh absolute position cannot be made wrong by a scroll that lands after it.

#### Passthrough exit paths

Prefix `d` detaches and exits 0; the daemon's `%exit` (graceful shutdown, `--stop`) reports "daemon exited" and exits 0; a closed socket (the daemon died, or the eviction policy disconnected the client) triggers one reconnect-and-requery — a fresh handshake plus a resync of the same pane — and exits 0 when that succeeds or the daemon is truly gone.
The terminal is restored (raw mode off, scroll region reset) on every path, including errors: attach never leaves a terminal wedged.

### Library API (feature `attach`)

Besides the passthrough client, the attach module exposes the pieces the render TUI is built on. The tmux layout-string parser (`attach::layout::parse_layout` / `parse_layout_triple`) maps a `%layout-change` triple to absolute per-pane rects, honoring the `Z` flag by rendering the *visible* layout — the zoomed pane alone — and pinning the same leaf order `list-panes -t` reports and `LayoutTree::render` emits (golden tests over real-tmux strings plus round-trips against the daemon's own renderer).
The pane renderer (the crate-private `render::PaneRenderer`) mirrors every visible pane in its own core `Terminal` fed by the `refresh-client -t` replays and `%output` bytes, maps cells to a ratatui `Buffer` (truecolor passthrough, wide-char spacers, combining marks), draws dividers with a focused-pane highlight (UTF-8 box drawing, ACS fallback), and damage-diffs frames at a 16 ms cadence so an output flood between two frames collapses into one diff.
`run` uses the default mode (`AttachMode::Render`); `run_with_mode(options, AttachMode::Passthrough)` selects the byte pump.

### Render mode (`--mode render`)

`par-mux attach` defaults to the pane renderer; `--mode passthrough` selects the passthrough byte pump instead, and an unknown `--mode` flag value is a loud error naming the valid modes (a silent fallback once ran a typo'd session in the wrong mode).
With no `-t`, the target-less rule runs (both modes): a daemon with no sessions gets the default tree created (`new-workspace -n default` — workspace and session `default`, a window named `1`, one unnamed pane) and that session attaches; exactly one existing session attaches as-is; several are a refusal that lists them with the `par-mux attach -t <session>` hint (exit 1) — the user picks, the client does not.

**Session picker (`[client] session-picker`, default off).** With the key on, the several-sessions case asks instead of refusing: before the attach takes over the screen (both modes), the client draws a compact list inline on the main screen — a bold title (`par-mux: several sessions are running — choose one`), one row per session in `list-sessions` order with its id, name, and window count (dim) in aligned columns, the selection `>`-marked and bold, and a dim footer of keys. `↑`/`↓` or `j`/`k` move (wrapping), `g`/`G` (or Home/End) jump to the first/last row, Enter attaches the selected session, and Esc, `q`, Ctrl-C, or Ctrl-D cancel: the list erases itself and the same refusal prints (exit 1), so the `-t` syntax stays on screen. Names are stripped of control characters and every line is cut to the terminal width; a list taller than the terminal scrolls with the selection. The picker runs only when both stdin and stdout are terminals, so a piped or redirected attach (scripts, CI, agent shells) keeps the plain refusal whatever the setting says. The list is read over a plain control connection closed before the attach handshake, so a picker left open never counts as a viewer (no window re-fit, no `done` cleared). It reads the setting at launch; the settings dialog's `general` tab toggles it for the next attach. An explicit `-t`, zero sessions (the default-tree create), and one session behave as before (pinned by `picker_runs_only_with_the_setting_and_both_stdio_terminals`, `picker_state_moves_wraps_picks_and_aborts`, `frame_lists_aligned_rows_with_the_selection_marked`, `picked_target_stays_out_of_the_way`, `session_picker_off_or_without_a_terminal_keeps_the_refusal_bytes`, `session_picker_needs_both_stdin_and_stdout_on_a_terminal`, `session_picker_pick_attaches_the_chosen_session_render`, `session_picker_pick_attaches_the_chosen_session_passthrough` and `session_picker_abort_prints_the_refusal_and_exits_nonzero`).
Every user-initiated landing (window/session/workspace switches, tab and picker clicks, the new-window landings, the closed-tab survivor) goes through `switch-client -t <window>`, so the displayed session — persisted, and what the follow broadcasts read — always names what the user last landed on.
The client enters the alternate screen, mirrors every visible pane of the window in its own core emulator, and paints the layout at 16 ms frame cadence.
The host terminal captures the mouse (SGR + button-event tracking), and stdin is routed rather than forwarded.

#### Keys

An incremental parser tokenizes the raw stdin stream; plain byte runs forward verbatim behind the prefix scan (prefix `d` detaches, `C-b C-b` sends the literal), while escape-sequence keys (arrows, Home/End, tilde function keys, SS3 forms, modifier chords, kitty CSI-u keys) are decoded and re-encoded against the FOCUSED pane's tracked input state — DECCKM application cursor keys, kitty keyboard flags, modifyOtherKeys — exactly what the pane's replay and `%output` set (the shared `keyboard::encode_key` encoder, the same one the daemon's `send-keys` uses).
A pane running vim gets `ESC O A` for Up; a plain shell gets `ESC [ A`. Full kitty keyboard protocol forwarding (negotiating flags on a pane's behalf) is not implemented; incoming host CSI-u keys decode through the same key path and re-encode per the pane's tracked state.

#### Mouse

SGR reports from the host are located in the layout rects. A click on the TAB STRIP (top row) switches to that window (see below); a RIGHT press on a tab opens that window's context menu (`rename` / `close` / `add tab` / `move left` / `move right` / `move to start` / `move to end` — the move rows send `move-window -s {id} -t {slot}` for the right-clicked tab from the client's cached window order, a no-op move sends nothing, and an edge left/right flashes `already the first/last tab` exactly as the move chords do; modal: esc/q closes it, a click on an action row dispatches, a click elsewhere is consumed; in every context menu arrows/`j`/`k` move the selection (wrapping past the header and footer rows) and Enter dispatches the selected row — one subtle reversed-label highlight serves the keyboard's selection and the pointer's hover alike, the hover a no-button motion report writing the same index); a click in the content area focuses the pane under the pointer — locally (divider highlight) and daemon-side (`select-pane`, only when focus actually changes: a re-click re-selecting the pane broadcasts a `%layout-change` and repaints the whole window) — and forwards a pane-relative report when that pane owns mouse tracking (its emulator tracked DECSET 1000/1002/1003 from the pane's own bytes), spelled SGR when the pane enabled 1006 and otherwise in the classic `ESC[M`/urxvt dialect it negotiated — an SGR report a pane never asked for is misparsed as keystrokes (the manual-pass htop report: htop under iTerm.app's bundled terminfo, whose `TERMINFO_DIRS` the daemon's panes inherit, enables only 1000, and the misparsed header clicks opened its sort-selection dialog instead of sorting).
Drag and release forward the same path, gated by what the pane's mode asked for: the host captures in any-event mode, but motion reaches the pane only when it negotiated it — drag motion needs DECSET 1002+, hover motion 1003; release is part of 1000. A pane without mouse ownership consumes nothing and its wheel scrolls the client. Over such a pane a left-button DRAG selects text instead: the press arms (a click — nothing highlights, focus still follows), the first motion promotes it to a character sweep painted REVERSED as it extends, and release copies the swept text (the anchor-to-head span in reading order, each row trimmed, rows joined with newlines) to the host terminal's clipboard as an OSC 52 write and clears — the same write scroll mode's `y` uses. On a pane that DOES own the mouse, plain drags forward to the pane and select nothing; SHIFT overrides the pane (the tmux escape hatch) — a shift-drag selects there too. The pointer's cell mapping is the paint path's absolute line (scrollback lines first, then live rows), so a drag works on a scrolled view as well.

#### Wheel

Over a pane that does NOT own mouse tracking, the wheel scrolls the CLIENT's view of that pane into its scrollback (3 lines per notch, clamped to history; the rect's top rows show the newest history lines and the live screen shifts down). A scrolled-off-live view is HELD against pane output — new lines push into history under the pinned view instead of snapping it back to the bottom, so the wheel works over a pane that keeps writing (a streaming agent chat, a log tail); the view releases when you scroll back down to the bottom, and any forwarded keystroke still snaps back to live. Over a pane that DOES own the mouse, the wheel forwards pane-relative like any other mouse event (the pane scrolls itself).

#### Status bar (bottom row)

The window renders into the rows between the tab strip and a reserved bottom row carrying `workspaces | sessions | windows | pane title` — the workspaces as names in id order (the daemon's active one bold), every session as `$N:name` (the shown one bold), the shown session's windows as `N:name` (the active one bold and `*`-marked), and the focused pane's title. The agent roster no longer rides the bar: the side panel's agents section renders one `{workspace} · {agent} ({state})` row per `list-agents` entry.
The state is the throttled re-query the notifications teach: `%sessions-changed`, `%workspaces-changed`, `%session-renamed`, `%window-add` and `%window-close` mark it stale, and one re-query per burst repaints the row. Against a daemon advertising `client-snapshot v1` that re-query is a single `client-snapshot -t <window>` round trip, and older daemons get the per-fact `list-workspaces`/`list-sessions`/`list-windows`/`pane-title`/`list-agents` round. `%window-renamed` and `%pane-title-changed` patch the row from their own payload without a query. `%agent-state-changed` and `%agent-released` patch the client's cached agent roster the same way, with no query. The bar does not show agents, so the patch repaints the side panel's agents section (see [Workspace side panel](#workspace-side-panel-s)). An agent row for a pane the cache has not seen yet also marks the state stale, so the throttled re-query fills the row's workspace. Passthrough mode (`--mode passthrough`) intentionally keeps its single-line form: its status row still ends with ` | N agent(s)`, and an agent notification marks the facts stale and re-queries instead of patching (passthrough has no side panel to carry the roster).
Render mode's daemon commands are pipelined (ENH-049): fire-and-forget chords and forwarded keys are written without waiting, reply-consuming commands and chains park a continuation that the pump dispatches at its ~16 ms frame tick, and a chain's landing is dropped if a newer navigation started while its reply was in flight — so the pump keeps reading input, painting frames, and draining notifications regardless of daemon latency, and a wedged daemon shows the "daemon busy" flash instead of freezing the UI. The deliberately synchronous exceptions are the startup reads and the seed/reseed/resize internals (their correctness depends on reading the reply before the notifications it queued), all bounded at the 1 s interactive budget; the status refresh — the pump's throttled re-query and the seed's seeding read alike — is not one of them any more: the snapshot is written without waiting and its reply applies as a pump-tick continuation.
A gone view lands up the tree instead of ending. When the shown WINDOW is gone (its last pane exited — auto-removed under `remain-on-exit = false` — or was killed) and its session survives, every client showing it lands on the session's active window (tmux semantics; pinned by `render_mode_ctrl_d_closing_a_tab_keeps_both_clients_in_the_session`).
When the shown SESSION is gone, the client lands on the active window of its workspace's landing session (the displayed session when the workspace holds it, else the first listed; pinned by `render_mode_ctrl_d_on_a_sessions_last_pane_lands_on_the_sibling_session`); when the WORKSPACE is gone too, on the daemon's active workspace's (pinned by `render_mode_kill_of_the_shown_workspace_lands_on_the_surviving_one`). The attach's own seed lands too: a target that resolved and was then torn down before the first frame lands on a survivor. The seed has no session yet, so its walk starts at the daemon's active workspace rather than the gone window's session (pinned by `seed_lands_on_the_survivor_when_the_target_dies_before_its_size_report`), while a `-t` target that does not exist when the client first asks is an attach error. On the pipelined async path (the snapshot reply's `SessionGone`) the landing resolves from the client's cached window list first — the cached active window when it survives the gone one, else the first listed sibling — and walks the interactive ladder only when the cache holds no survivor or the cached landing died before it could be shown, so a pathological daemon parks nothing extra on the landing path (pinned by `a_gone_window_lands_on_the_cached_sibling_without_queries` and `a_dead_cached_landing_falls_back_to_the_interactive_ladder`).
The view ends only when the daemon has nothing left to show — no workspace with a session, which explicit kills and, by default, an app exit can still produce (pinned by `render_mode_kill_workspace_of_the_only_workspace_ends_the_client` and `render_mode_ctrl_d_on_the_workspaces_last_pane_cascades_and_ends`) — or the daemon exits. A pane closes when its child exits (Paul's rule, see [Pane Reaping](#pane-reaping)): a last pane's exit empties its workspace and the view lands on the next one, none remaining ends the client. The keep-one-tab hold survives as the `set-workspace-respawn` opt-in (see [Pane Reaping](#pane-reaping)). Passthrough mode keeps its single-pane behavior.
While the scroll viewport is up, a bold `[scroll +N]` cue reports the offset. A host resize repaints the row with the content (the same redraw discipline the pane frames follow, so a repaint flood cannot leave it blank).
The bar ends with a ` C-b? Help ` chip RIGHT-ALIGNED at the row's right edge — the live help keybind followed by the label, spelled the way the keybinds panel spells the chord; its width is reserved before the rest of the line composes AND the paint clips the content at the chip's left column, so a full line can never push the chip off the row.
While hidden, the bottom row is never written at all — the pane owns it (the refit's repaint erased the bar once). Prefix `S` (`[client] status-bar`) hides the bar — the pane grid grows the row, and the bar's vanishing is its own feedback, since a flash would paint onto the hidden row — and again restores it; the refit's full repaint invalidates both chrome rows, so the tab strip and status bar survive every refit (pinned by `render_mode_status_bar_toggle_hides_and_restores_the_row` and `render_mode_sidebar_toggle_repaints_the_strip_rows`).

#### Tab strip (top row)

One tab per window of the shown session — the bare NAME in window order, drawn as herdr's tab bar in truecolor: the whole row is a dark navy bar (`rgb(23,23,33)`), each tab is a solid block with two columns of padding either side of its name, and adjacent blocks are separated by exactly one bar column.
The active block is bright blue (`rgb(126,171,249)`) with dark bold text; inactive blocks are a lighter navy (`rgb(43,44,60)`) with muted text (`rgb(97,101,123)`); the `…` edge markers and the ` + ` paint muted on the bar.
A click anywhere on a block, pads included, hits that tab; gap and marker columns hit nothing. When the view is reseeded (a workspace switch, a resize, a sidebar toggle) the strip repaints every column, so a workspace with fewer tabs never leaves the previous workspace's extra tabs on the row.
The strip reads the SAME queried state as the status bar (the `list-windows -t` half of the throttled re-query), so window add/close/rename refreshes it with the status row.
Truncation keeps the active tab always visible (the par-term tab-bar rule): full names when they fit, otherwise every name truncated to the widest shared budget with an ellipsis, otherwise a contiguous run of tabs containing the active one with `…` markers at the hidden edges.
A click on a tab switches to that window through the same daemon-side `select-window` + resync every switch uses — a strip click never focuses or forwards into a pane. A press arms a drag: motion to another tab and a release there reorders the windows (`move-window -s @from -t <slot>`; the `%sessions-changed` refresh repaints the strip, and the wheel offset resets with the fresh order), while a release on the origin tab is just the click.
When the truncation run hides tabs at the edges, a wheel over the strip shifts the visible run toward the hidden tabs (3 tabs per notch, clamped to the list; a strip that fits ignores the wheel, and a re-seed resets the offset) — the TUI audit's R6. Wheeling from the active-centered run moves the run in whichever direction the wheel turns — earlier or later tabs — and the clamped edges stay put; prefix `n`/`p` (next/previous window) reach every tab by keyboard, the run re-centering on each landed tab. The keyboard gets a scroll-without-switching gesture too: prefix `H` / `L` (`[client] tab-strip-left` / `tab-strip-right`) shift the run toward the earlier/later windows — 3 tabs per press, clamped like the wheel, a strip that fits ignoring them, and the shown window never moving (pinned by `tab_strip_chords_scroll_an_overflowing_strip`).
With the side panel up the strip shares its row with the panel: the ` workspaces ` title paints accent in the panel's columns (clicks there do nothing), and the tabs lay out from those columns onward — the hit-test maps the raw host column through the offset.
A ` + ` button sits immediately after the last visible tab (or right `…` marker) — one blank column before its glyph — inside a three-column reservation capping the row's tail, so it never runs past the line at any width: a click opens the new-tab prompt — the next free index as the editable default (bumped past any window NAME claiming the number), `enter save · ^c clear · esc cancel`, Enter sending `new-window -t <session> -n <name>` and landing the view on the fresh window.
The content grid (what `refresh-client -C` reports and what clicks/wheels/cursor placement map through) is the host grid minus the strip row minus the status row.

#### prefix [ (scroll mode)

A keyboard viewport over the FOCUSED pane's client scrollback. The viewport starts one window up from live (clamped to the history extent), holds against pane output (new lines append below without snapping the view), and moves with Up/Down (a line), PgUp/PgDn (a page), Home (the top of history); q, Enter, or End exit and snap back to live.

The copy-mode extension (the TUI audit's R1) rides the same viewport: `v` anchors a LINE selection at the viewport's top (selected lines paint reversed) — scrolling extends the head, a second `v` clears; `y` copies the selected lines' text as an OSC 52 clipboard write (the host terminal's clipboard policy applies; the flash names the line count); `/` opens a search line on the status row (typed bytes edit it, Enter commits, Escape cancels), the first search collects the pane's matches case-insensitively, and `n`/`N` walk them with the viewport landing on each match (`match k of N` on the flash). Leaving scroll mode clears the selection, the query, and the match list. The client also bridges the FOCUSED pane program's own OSC 52 clipboard writes to the host (par-term's clipboard_sync contract, attach side): a pane copy — local or over SSH — updates the pane emulator's clipboard state, the client re-emits the same wire form on the frame's outbox when it changes (deduped per pane; a pane switch forwards the pane it lands on), and the host terminal's clipboard policy applies.
`e` opens the pane's scrollback + screen in `$VISUAL` (else `$EDITOR`; an empty value counts as unset) — herdr's `edit_scrollback`. The client writes the pane's plain text (soft-wrapped rows rejoined into logical lines) to an owner-only temp file, leaves scroll mode, and splits a temporary pane off the focused one that runs the editor under `/bin/sh` and is zoomed and focused; when the editor exits the temp file is deleted and the pane closes (under `remain-on-exit = true` it is held dead instead, like any exited pane). The file is a scratch copy: nothing is written back to the pane, so search, copy, or save it elsewhere from the editor. With neither variable set the key is refused on the flash (`edit scrollback: set $VISUAL or $EDITOR`) and scroll mode stays up. Unix only (Windows flashes a refusal).
Keys never reach the pane while the viewport is up. Typed letters (`v` `y` `/` `n` `N` `e` `q`) work as plain keystrokes. This is not full tmux copy-mode: selection is by whole lines and there are no cursor motions.

#### Management chords (`%`, `"`, `x`, `c`, `{`/`}`)

The same daemon commands the passthrough chords send, mirrored onto the render view: `%`/`"` split the FOCUSED pane (`split-window -t <focused> [-h]`), focus the fresh split (the reply body is its id), and re-seed the window from the layout the split broadcast — the new pane renders at its rect immediately; `x` kills the focused pane and lands the view on the window's surviving active pane (killing the window's last pane lands the view on the session's active window, or ends it when the session went with the window); `!` breaks the focused pane out into a new window of its session (`break-pane -s <focused>`) and lands the view on the fresh window; `J` opens the session/window picker in join mode — the picked window (a session header counts as its active window) receives the focused pane (`join-pane -s <focused> -t <its marked pane>`), and the view lands on the destination; `c` opens a window in the shown session and re-seeds onto it; `{`/`}` swap the focused pane with its layout-order neighbor (`swap-pane -s <focused> -t <neighbor>`; the swap's `%layout-change` re-seeds the window). `<`/`>` move the shown window left/right in the session's tab order (`swap-window -s <shown> -t <neighbor>`; the `%sessions-changed` broadcast repaints the strip, the shown window stays shown by identity, and an edge press flashes instead of wrapping). Passthrough carries `!` too; `J` needs the picker's overlay surface and is render-mode only there.
All rebind live on the reload chord, settable as `[client] split-right` / `split-down` / `kill-pane` / `break-pane` / `join-pane` / `palette` / `new-window` / `swap-prev` / `swap-next` / `move-window-left` / `move-window-right` / `tab-strip-left` / `tab-strip-right`.

#### Command palette (`:`)

Prefix `:` (settable as `[client] palette`) opens a filterable palette over every management action — the keyboard path to the click-only menu chips (the TUI audit's R2): `settings`, `keybinds`, `reload config`, `detach`, plus `new window`, `break pane out`, `join pane into tab`, `split right`/`down`, `kill pane`, `zoom pane`, `rename window`/`pane`, `workspace picker`, `toggle sidebar`/`status bar`, `next`/`previous workspace`, `move tab left`/`right` (their chords), and `move tab to start`/`end` (chordless; the shown window through the tab menu's move rows). The palette is the picker's machinery — title ` command `, each row carrying its action's EFFECTIVE chord at the live binding, spelled the way the keybinds panel spells it (the chordless settings row carries none, and a remapped chord shows its remapped key; the filter matches the whole row, so a chord fragment finds its action), the filter live from the open — typing filters at once with no `/` first, arrows move the selection without leaving the filter, Enter runs the selected action, esc dismisses — and the four non-chord actions run their own paths (settings opens the dialog, keybinds the help panel, reload takes the reload chord's rebind, detach ends the client). Render mode only: passthrough consumes the chord with a flash naming why.

**Exit cue and respawn (`r`, `[client] respawn`).** A pane the daemon holds dead — a live `%pane-exited`, or the registration replay's line for a pane already held when the client attached — puts a bold cue at the head of the status bar: `(exited N — C-b r respawns)` when it is the focused pane (`?` for an unknown code), `(%N exited N)` when another pane of the shown window is the held one. Prefix `r` sends `respawn-pane -t <focused>` only while the focused pane is held — a live pane gets a `pane is running` flash and nothing is sent (never `-k`) — and a refused respawn flashes the daemon's error. The `%pane-respawned` broadcast clears the cue and gives the pane a fresh emulator plus a replay, so the dead process's frozen screen never mixes with the new shell's output. The chord ships in the settings dialog's `keys` tab (`respawn dead pane`) and in the help panel at its live binding (pinned by `exit_cue_and_respawn_chord_follow_the_held_pane`, `render_mode_held_pane_shows_the_exit_cue_and_the_chord_respawns_it`, `render_mode_pane_held_before_attach_shows_the_exit_cue` and `render_mode_settings_keys_tab_lists_the_respawn_chord`).

#### Zoom, rename, and border-style chords (render mode)

`z` zooms the focused pane through the daemon's `resize-pane -Z` — the pane's PTY takes the full window grid (less any declared per-pane chrome, see `refresh-client -I`), the `%layout-change` broadcast re-seeds the view, selecting away or any layout mutation unzooms (the daemon's zoom rules), and a bold ` Z ` cue marks the status row while the zoom is on (`[client] zoom`; passthrough consumes the chord unbound).
The cue tracks the daemon's per-window zoom truth, not just this client's chord: every `%layout-change` for the shown window sets it from the `Z` raw flag, so another client's `resize-pane -Z` — or the select-away/layout-mutation unzoom — flips every attached render client's cue (the chord still flips it locally for instant feedback, and the broadcast agrees).
`,` opens a modal rename prompt seeded with the window's current name; typing edits it (Backspace pops, Enter commits, Escape cancels) and Enter sends `rename-window -t <window> <name>` (with a `--` before a name whose first word is `--`) — the tab strip and status refresh carry the new name (`[client] rename-window`).
`$` is the same prompt for the focused pane's sticky user title, committing as `select-pane -t <pane> -T <title>` (`[client] rename-pane`).
`B` cycles the border styles — unicode → double → heavy → ascii → herdr — repainting at once; herdr draws every pane its own complete rounded box (the other four the shared dividers), and an explicit `pane-borders` still overrides the paint mode for any glyph set (`[client] border-lines`, default `herdr`, live on reload).
`l` toggles the pane labels — each pane's EFFECTIVE title embedded in its top border edge (`[client] label-toggle`; labels paint in the per-pane-box modes, `pane-borders` or the herdr style).
The label is the daemon's `pane-title` value: the user `-T` label (prefix `$`) when set, else the pane's OSC title; the client re-queries on the throttled status refresh, so a rename paints within a beat.
Prefix+arrow keys select the nearest pane in the arrow's direction, and Shift+arrows swap with it (`swap-pane`; focus follows the pane's content through the swap's `%layout-change`) — the prefix-arrows are a fixed binding, not rebindable; while zoomed they aim through the zoom, and the daemon unzooms on the select.
The status row's tab strip is herdr-shaded: the active workspace/tab a solid block, the rest dim, `|` between tabs.

#### Pane resize

Prefix `R` (settable as `[client] resize`) enters a sticky resize mode — arrows send `resize-pane -t <focused> -L|-R|-U|-D <step>` at the configured step (`[client] resize-step`, default 1) until Enter/Escape/`q` exits and any other key leaves the mode.
Render mode adds the mouse: a press ON a divider cell — the painted divider column/row itself, never the pane content cells beside it — starts a drag (it never focuses or forwards — a divider press is not a click-through), motion maps the pointer's cell delta to the same relative `resize-pane` aimed at the boundary's left/top pane, applied at most once per 80 ms (every step is a daemon layout change that repaints the whole window, and pointer motion arrives far faster than that should happen; the unapplied remainder is flushed on release, so a drag lands exactly where it is let go), with the dragged divider drawn reversed while the drag lasts; a release that never moved falls through as a focus-only click (the pane under the pointer still focuses; the release itself is swallowed — the press was consumed as a divider grab, and a lone release would hand the pane app a phantom click).
The prefix chords work in passthrough too (the wire command only; the daemon's repaint rides the pane's `%output` stream).

#### Help panel (`?`)

Prefix `?` (settable as `[client] help`) opens the bindings modal — a themed modal (every cell in the resolved host background, rounded box-drawing ring in the accent color) titled `keybinds` with an `esc close` badge embedded in the top border and a footer line `search / · scroll j/k/arrows/pgup/pgdn · close esc/enter`.
Entries group into category headers (`global`, `workspaces`, `windows/sessions`, `panes`, `navigation`, `mouse` — the round-7 order), each row showing the chord at its EFFECTIVE binding (a remapped chord shows its remapped key, from the live config). the filter line appears the moment `/` opens it (a live ` /query▌` cursor; Backspace edits, Enter commits the filter — the footer already advertises `search /`, so an idle panel carries no placeholder row), j/k and the arrows and PgUp/PgDn scroll the content window when it overflows, and esc/Enter/q close — the prior frame repaints.
When the content overflows, a `┃` thumb on the ring's right column tracks the scroll window. Keys AND the mouse never reach the pane while the panel is up: wheels scroll the panel.
Passthrough prints the same category rows as plain text (no overlay surface; the pane's next output redraws over them).

#### Session/window picker (`w`)

Prefix `w` (settable as `[client] picker`, default `w`) opens the picker modal — the same themed chrome as the help panel (resolved host background, accent ring, `esc close` badge, the controls footer on a dark-grey band) titled `picker`: one accent header per session (`$N: name`, `>`-marked when the view shows it) with its windows nested beneath (`@N: name`, `>`-marked for the shown window, `*` for the session's active window).
`/` opens a filter-as-you-type box (the help panel's control; headers hide when nothing beneath them matches), arrows/`j`/`k` move the selection cursor (wrapping), and Enter (or a click on a row) activates through the same select+resync contract every switch uses: a session header lands on its active window, a window row on that window (`switch-client -t`, falling back to `select-window -t` on a daemon without it, then the renderer re-seeds). esc/`q` dismiss and the prior frame repaints; keys AND the mouse never reach the pane while it is up (wheels move the selection).

#### Workspace picker (`g`)

Prefix `g` (settable as `[client] workspace-picker`, default `g`) opens the picker's workspace sibling, titled ` workspaces `: one row per workspace (`+N  name`, `>`- and `*`-marked when active), the same filter/cursor/click/footer modal machinery, and Enter landing on the workspace through the same select-then-resync (`select-workspace -t`, then the session the workspace resumed — its active session, the one `select-workspace` displays — lands through `switch-client` and its active window re-seeds). esc/`q` dismiss; keys and the mouse never reach the pane while it is up.

#### Workspace side panel (`s`)

The panel is shown when a render client attaches (`[client] sidebar-on-launch`, default `true`; `false` starts hidden). The launch state is applied before the client's first size report, so the daemon's first division already reserves the strip; it is read at launch only, and a config reload leaves the live panel as it is.
Prefix `s` (`[client] sidebar`, width `[client] sidebar-width`, default 20, clamped 6–60) toggles a left strip — herdr's sidebar: a dim right-edge divider, themed-bg fill, and two SEPARATED areas (herdr's panel shape): the workspace area takes half the panel's rows by default and opens directly with its rows (its title rides the tab strip's lead segment), the agents area opens with its dim bold header row (` agents`, the row's right edge carrying the roster's sort toggle — a clickable `grouped`/`priority` label naming the mode in force; a click flips the ordering in place and persists `[client] agents-sort` through the settings write path), and a full-width rule divides them. Dragging the strip's right-edge divider resizes the panel (12–60 columns; the re-reported grid follows at the pane drags' throttled cadence, the release landing the final width); dragging the rule row resizes the areas — the split is held as the workspace area's share of the rows, so it survives a host resize, and each area keeps at least three rows (a local recompose, no daemon round trip). The `«` chevron at the panel's bottom-right collapses it to herdr's 2-column status rail: per workspace its 1-based index (`+` past nine) and its agents' most urgent state (question > blocked > done > working > idle: `◐` working, in yellow; `●` done, blocked, or question, in cyan, red, or magenta; `○` idle, green; `·` no agent), the rule at the same split, per agent its index and state glyph, and a `»` chevron on the last row that expands it again. The rail has no divider column and no width handle; its entries stay clickable, and its tab-strip lead stays blank. Collapsing re-reports the narrower grid like a toggle. The dragged width, the split, and the collapse state persist in `client-state.json` beside the config file (written on each drag release and chevron press through an atomic rename, read at attach launch, where they override `sidebar-width`); prefix `s` still hides and shows the panel in whichever state it is in (pinned by `sidebar_chevron_collapses_to_the_rail_and_persists`, `compose_sidebar_rail_shows_indices_statuses_and_the_expand_chevron`, `compose_sidebar_split_ratio_moves_the_rule_and_clamps` and `render_mode_persisted_collapsed_panel_launches_as_the_rail`). The workspace rows come first: the queried workspace roster, one row per workspace (windows live in the tab strip, not the panel). Only the DISPLAYED session's workspace is highlighted (each workspace row carries its agents' status glyph directly after the workspace NAME — `◐` working, `●` done/blocked/question, `○` idle, and nothing when no agent state is known — painted in the state's color; the marker column stays the active pointer) (the daemon's active one when the session carries none — the initial `attach -t` shows the session's windows, so the highlight names the session's own workspace; the TUI audit's F1): herdr's subtle full-width band — the host background lifted a step (`#1c1c29` over black, darker on a light theme, herdr's `#1e1e2e` when the background probe failed) — with its first line in the full foreground and bold, never a reversed block (pinned by `sidebar_active_entry_paints_a_subtle_band`). A workspace-row press lands on the workspace, and a workspace-row RIGHT press opens that workspace's menu (`rename` / `close` / `new` — the same modal vocabulary as the tab menu). The agents section follows: one entry per `list-agents` row, ordered by the roster's sort mode (`[client] agents-sort`, the header's toggle): `grouped` (the default) keeps the workspace groups in id order with the agents beneath by status priority (the same question > blocked > done > working > idle order as the glyphs), panes ascending as the tiebreak; `priority` is herdr's attention queue — status first, most urgent at the top, then workspace, then panes (pinned by `grouped_mode_subsorts_agents_by_status_priority` and `priority_mode_sorts_by_status_then_workspace`). An entry is one panel line per `[client] agent-rows` line (herdr's `[ui.sidebar.agents] rows`), each joining its tokens with ` · `: `state_icon` (the state's colored glyph, the half-circle `◐` when working), `state`, `workspace` (the workspace name), `project` (the host probe's `project` field — the probe sweeps every 30 s and broadcasts nothing, so while the panel is up and an agent's project is unknown the client re-queries the roster every 10 s until it arrives), `message` (the state's message — the blocked reason, when the hook reported one; the `verbose` preset shows it as the entry's second line), `agent`, and `pane`; a token with nothing to show drops out, an unknown token is skipped with a warning (stderr at launch, the status flash on reload), and the default `[["state_icon", "workspace", "project"], ["agent"]]` reads `● main · par-term` over `claude`. The project poll is pinned by `missing_project_polls_the_roster_until_it_arrives`. The entry whose pane holds the view's focus takes the same band across all its lines, its later lines staying dim and its state dot keeping its color, and a click on any of its lines lands on the pane (pinned by `agent_entry_lines_render_the_token_rows`, `compose_sidebar_lays_out_multi_line_agent_entries` and `sidebar_agent_entries_follow_agent_rows`). Clicking an agent row sends `select-pane -t %N`: a pane in the shown window just takes focus, and a pane in another window re-lands the view on that window (the select+resync contract). An entry whose workspace is unknown (a pre-`ws=` daemon, or a pane first claimed since the last status refresh) drops its `workspace` token and trails in a group after every workspace group. The section is rebuilt from the cached roster the moment `%agent-state-changed` / `%agent-released` arrives, with no query, so a working→blocked transition shows on the next frame; the attach seed re-installs it once its status refresh commits, so a re-attach with quiet agents opens populated (pinned by `the_seed_installs_the_agents_section_after_the_roster_lands`). The throttled status refresh that the insert marks stale then fills a newly claimed pane's workspace and moves its row into its group. Pinned by `sidebar_agents_section_groups_by_workspace`, `sidebar_agents_group_by_workspace_id_not_name`, `agent_insert_marks_status_dirty_and_rows_trail_ungrouped`, `sidebar_agent_row_click_focuses_the_pane`, `agent_event_patches_without_a_query` and `render_mode_side_panel_carries_the_agent_roster`.
While it is up the daemon divides the REDUCED grid (`cols - width` — the strip is a client-only overlay it never learns of), every pane, divider, border, label, and cursor carries the strip offset exactly ONCE — pane content insets were rect-relative and `display_rect` carried the offset, so a stacked second application pushed pane content the width again (content doubled right, clipped at the frame edge, dead margin between divider and content; pinned at the renderer level by `sidebar_offset_paints_content_and_dividers_together` and end to end by `render_mode_sidebar_toggle_refits_pane_geometry`, which parses the CUP column a marker paints at before/during/after the toggle).
Toggling refits the grid on the pump and its full repaint erases the region the old layout vacated. The top status row switches to herdr's compact form while the strip is up: the ACTIVE workspace's label — the one full-width-shaded run on the line, agreeing with the strip's highlighted row — then the shown session's tabs, the active tab bold and the rest dim (the tab's solid block moved to the label: with both shaded, the line read as selecting another workspace).
The roster re-queries on open and on every throttled status refresh while up (`%workspaces-changed` rides it); every re-seed path (window switches, panel-row clicks, the split and kill chords) re-fits the live renderer in place at the HOST grid — never the reported extent, or each re-seed ratchets the view one panel-width narrower — so the chosen border-glyph set, the session-level pane-borders flag (the cycle chord flips the session flag, not just the renderer's), and the panel width/rows all carry over, and only panes new to the renderer replay (ENH-044; pinned by `render_mode_border_style_survives_split` and `render_mode_workspace_click_reseeds_with_the_panel`).
The workspace area ends with a footer row (herdr's spaces footer): a ` new ` chip bottom-left opening the new-workspace prompt (the next non-conflicting workspace index as the editable default, commit sending `new-workspace -n <quoted name>` then `select-workspace` and landing the view on the fresh workspace — the daemon spawns the new workspace's first session/window, so the landing has a live window to reach) and a ` menu ` chip at the area's bottom-right opening the command menu beside it — the box's left edge just right of the chip, its bottom edge on the chip's row, pulled back inside the frame when it would overflow — (`settings` opens the settings dialog below, `keybinds` opens the keybinds panel, `reload config` runs the reload chord's rebind plus the daemon's `reload-config`, `detach` ends the client like prefix `d` — the same modal as the context menus, rows clicked at their raw host columns, the selection moved by arrows/j/k and written by the pointer's hover alike). The menu's anchoring is pinned by `menu_chip_opens_the_command_menu` and `anchored_overlay_stays_inside_the_frame`.
The workspace menu (reached by a right press on a workspace row) carries `rename` via `rename-workspace -t <id> <quoted name>`, `close` via `kill-workspace -t <id>` — killing the SHOWN workspace lands the view on the first surviving workspace, none remaining ends the view cleanly — `new`, and `respawn on exit`, which flips the workspace's keep-one-tab respawn option (`set-workspace-respawn -t <id> toggle`) and flashes the resulting state on the status row (pinned by `workspace_menu_respawn_row_toggles_and_flashes_the_state`). A refused close flashes the daemon's reason on the status row. The group rule's refusal (a repo workspace with open linked worktree workspaces, see [Git worktree workspaces](#git-worktree-workspaces)) flashes the linked count and the `-g` remedy instead (`app: 2 linked worktrees - menu 'close group' sends -g`), and for the next 10 s that workspace's menu shows `close group` in the `close` row: pressing it sends `kill-workspace -t <id> -g`, landing the view when its session lived in any group member. Any menu press spends the offer, so it never lingers into a later close (pinned by `refused_workspace_close_flashes_and_arms_the_group_close`).
Linked worktree workspaces group under their repo workspace's row: each follows its repo row in id order with a `├─ ` / `└─ ` prefix, labelled by the part of its name after the repo workspace's `<name>/` (so the default `app/feat/login` reads `└─ feat/login`). The grouping is `list-worktrees`' facts, carried on `client-snapshot`'s `worktree` rows, never derived from names, and every panel rebuild (the status refresh, and the agent notifications' no-query rebuild) applies it. A linked workspace whose repo workspace is closed keeps its plain id-order row, and a daemon without the `worktree` rows (the legacy per-fact refresh) shows no grouping (pinned by `sidebar_groups_worktree_workspaces_under_their_repo`).

Every workspace row carries its git badge: a bright-blue ` main ↑2` continuation line under it — the workspace's active pane's repo branch with its ahead/behind against the upstream (`↓n` behind), resolved daemon-side by the badge refresher — a background thread that runs one bounded (2 s) `git status -sb` per workspace on a 2 s tick, refreshing entries older than one second under the poll (`git-poll-secs` - 1) and pruning what the workspaces leave behind; a `client-snapshot` reply serves the cache ALONE and never spawns git (a snapshot costs map reads, and its git rows are as fresh as the last tick), with the badge's chain read under a brief tree lock: the active pane's live cwd (OSC 7, else the child's kernel cwd), falling back to the workspace's launch folder (its worktree checkout) when the pane reports none; no row, and no badge, for a workspace the refresher has not ticked on yet, a non-repo, a detached HEAD, or `[client] git-status = false` (default `true`, live on reload, a `general`-tab settings toggle). One row per workspace, so every workspace shows its badge whether or not it is the displayed one. The status stream carries no event when a repo changes, so while the panel is up and the key is on the client re-queries on a configurable cadence (`[client] git-poll-secs`, default 10; 0 disables polling, otherwise clamped 3-600 — never below the 2 s git run — a sidebar-tab settings row, live on reload); the client's status re-query is bounded by the 500 ms status timeout, and a timed-out re-query waits for the next poll or event instead of re-issuing immediately (an immediate re-issue queues behind the same slow reply — the freeze-storm rule). Pinned by `git_status_defaults_on_resolves_and_reload_keeps_it`, `apply_snapshot_commits_the_git_row_and_clears_without_one`, `git_badges_land_on_their_workspace_rows`, `client_snapshot_git_badge_follows_the_active_pane`, `client_snapshot_git_badge_falls_back_and_respects_the_key` and `client_snapshot_serves_git_from_the_cache_alone`.
With the panel up the TAB STRIP paints the panel's ` workspaces ` title accent in the panel's columns and lays the tabs out from them onward with ` + ` pinned at the strip's right edge, hit-testing the RAW host column through that offset — the panel-up top row stays clickable coherently (the round-5 plus-defect: the content rebase ran before the strip branch and shifted every panel-up top-row click one panel width left, leaving ` + ` unclickable — pinned red-to-green by `render_mode_plus_click_works_with_the_panel_up`).
Toggling reports the restored grid, so panes re-divide back to the full width.

#### Settings dialog

The command menu's `settings` row opens herdr's tabbed settings dialog over the panes — one row per `[client]` key that faces the TUI: a tab row (`sidebar`, `borders`, `general`, `notify`, `states`, `keys`, `integrations`), the section's heading and description, and its rows — each key's label with the value in force right-aligned, `▸` on the selection (a reversed band). Tab / `l` / `h` switch sections (a click on a tab too), `↑` / `↓` / `j` / `k` move the selection, and Enter, space, or `→` / `←` on a discrete row applies it (a click on a row selects and applies): toggles flip (`sidebar-on-launch`, `pane-borders`, `show-label-in-border`, `scrollbar-gutter`, `drag-cursor-shape`, `kitty-graphics`, `session-picker`, `git-status`, `sound`), choice lists cycle (`border-lines`, `theme`, `theme-dark`, `theme-light`, `notify-delivery`, `mode` — the attach pipeline, applied at the next launch — and the sidebar's `agent-rows` as named layouts: `default`, `compact`, `detailed`, `verbose`; `agents-sort` — the roster's `grouped`/`priority` ordering; `→` steps forward, `←` back), and numbers, hex colors, and chords (`sidebar-width`, `git-poll-secs`, `pane-gaps`, `resize-step`, `border-active-color`, `border-color`, the `prefix`, and every management chord in the `keys` tab, which scrolls) and the `notify` tab's sound texts (`sound-done`, `sound-blocked`, `sound-player`, and `sound-mute` as a comma list; an empty commit clears each) open the prompt modal seeded with the value in force — on those rows `→` / `←` still switch sections — the commit validates before writing (a bad number range, malformed `#rrggbb`, or a chord that does not parse flashes on the status row and writes nothing; an empty hex clears the override to the built-in) and Escape cancels back to the dialog. The `general` tab ends with one row per `[[client.popup]]` entry plus `+ add popup`, edited in the prompt as `KEY [WxH] COMMAND` (see [Popups](#popups)). The `states` section stays a multi-select toggling one entry of `notify-states` (`[x]` / `[ ]`). Applying writes the one key into the config file in place — `toml_edit` keeps the user's comments, layout, and every other key, a file that does not parse is never overwritten — then runs the reload chord's path, so the client rebinds from the file it just wrote and the daemon re-reads it (`sidebar-width` also applies to the live panel like a drag release). The `integrations` section lists the agent session hooks' install state, and Enter (or `u`) is its update button: it reinstalls every integration whose installed asset is not the bundled copy (`v1 → v2 outdated`, or `asset missing` under a surviving registration), leaving never-installed rows alone — the outcome flashes on the status row (`integrations: updated claude, codex` / `everything current`, or the failures with reasons) and the rows re-grade in place (pinned by `update_outdated_reinstalls_only_stale_installed_integrations` and `integrations_footer_offers_the_update_outdated_button`); install fresh integrations with `par-mux --install-agent-hooks` / `--install-agent-extensions`. Escape or `q` closes, and so does a press anywhere outside the box (pinned by `tabs_and_rows_fit_the_panel`, `row_sections_show_labels_and_values`, `keys_section_scrolls_and_spells_chords`, `settings_rows_write_and_apply_live`, `settings_prompt_rows_edit_numbers_hex_and_chords`, `settings_keys_section_scrolls_to_every_row`, `command_menu_settings_opens_the_dialog_and_applies`, `set_client_key_preserves_the_user_file` and `render_mode_settings_dialog_opens_from_the_panel_menu`).

#### Agent state notifications

A render client announces an agent's arrival in a selected state (herdr's background toasts). `[client] notify-states` picks the states (`working`, `blocked`, `question`, `done`, `idle`; default `["blocked", "done"]` — `question` (a dialog asking the user to choose) notifies wherever `blocked` would — needs attention, and finished while out of view; the daemon's seen flip `done` → `idle` never notifies) and `[client] notify-delivery` the channel: `inside` paints herdr's in-app toast — a bordered card at the pane area's bottom-right with the state's colored dot and a bold `{agent} {state}` title over a dim `{workspace} · {agent}` line, plus the state's message (the blocked reason) as a dim third line when the push carried one, up for 8 s when blocked and 5 s otherwise, a newer one replacing it, and a click on it focusing the agent's pane; the card also rings the terminal bell (BEL with the next frame) so the host flags the tab or window unread — `terminal` (the default) writes the host terminal's notification escape with the next frame — kitty's OSC 99 under kitty, OSC 9 elsewhere (iTerm2, Ghostty, WezTerm) — `system` hands it to the OS notifier (`osascript` on macOS, `notify-send` on Linux; argv only, detached, never waited on), and `off` disables them. The notification reads `par-mux: {workspace} · {agent}: {state}`. Only a pushed `%agent-state-changed` whose state differs from the last one the push stream announced for that pane notifies, and only for an agent OUTSIDE the window the client shows — herdr's background-only rule: the agent you are watching is already in view. The status refresh replaces the roster wholesale and never notifies, so attaching or reconnecting does not replay every agent's state; the first roster the client commits seeds the push baseline (minus any pane whose push is already queued), and later refreshes never move it — a refresh that already shows a new state (the daemon applies a report before it broadcasts it) does not swallow that state's push, and a push that arrives while the client is still seeding its window is handled once the view is up. An unknown `notify-delivery` keeps `terminal` and an unknown state is ignored, each with a warning (stderr at launch, the status flash on reload) rather than a refused attach (pinned by `render_mode_background_agent_state_notifies_the_host`). Agent and workspace names are pane-controlled, so ESC, BEL, and ST are stripped before they ride inside the escape (pinned by `agent_state_push_notifies_selected_transitions_only`, `terminal_notification_escapes_are_sanitized` and `notify_keys_resolve_defaults_explicit_and_warn_on_unknowns`). The toast is pinned by `inside_delivery_shows_a_toast_that_expires_and_clicks_to_the_pane` and `render_mode_inside_delivery_paints_a_toast`. Kitty is detected from the attach's environment as `KITTY_WINDOW_ID` set with `TERM_PROGRAM` unset or `kitty`: the window id leaks into processes started under a kitty window (a ghostty or iTerm2 launched from a kitty shell), and a host overwrites `TERM_PROGRAM` with its own name, so a leaked id no longer turns OSC 9 into an OSC 99 that ghostty and iTerm2 drop (kitty accepts OSC 9 too, so a miss there costs nothing; an attach inside a par-term or par-mux pane, which exports `TERM_PROGRAM=kitty` with the id, still gets OSC 99, which the core parses; pinned by `host_is_kitty_only_when_the_window_id_is_not_leaked` and `terminal_delivery_shapes_are_pinned_per_host_not_per_environment` — the render-session tests set the resolved flag explicitly, so the suite asserts the same shapes whatever terminal launched `cargo test`). What the host does with the escape is its own policy: Ghostty presents an OSC 9 only when its surface is NOT the focused one (no setting changes this), so a notification raised while you look at the attach's window is dropped by Ghostty itself; iTerm2 posts it when the profile's "Send escape sequence-generated alerts" is on and "Suppress alerts in active session" is off; both need macOS notification permission for the terminal app. A host baseline: `sleep 5; printf '\e]9;baseline\e\\'`, then switch to another app.

**Jump to the notification's pane (`j`, `[client] notify-jump`).** Prefix `j` (herdr's `open_notification_target`) focuses the pane behind the most recent agent notification THIS client delivered, through any channel, and dismisses the toast if one is up: `select-pane` plus a `pane-info` lookup, landing the view on the pane's window — in another session or workspace too — the same path a toast click takes. A transition the client did not announce (an agent in view, an unselected state, delivery `off`, the seen flip) never becomes the target, and with nothing delivered yet the chord flashes `no notification to jump to` and sends nothing. The jump CONSUMES its target (herdr consumes the visible notification and promotes the next queued one): repeated jumps walk back through earlier notifications — newest first, at most the last eight distinct panes — instead of re-landing on the same pane until something newer arrives. herdr's default `prefix+o` is not taken because prefix `o` cycles panes here. The chord ships in the settings dialog's `keys` tab (`jump to notification`), the help panel, and the command palette (pinned by `notify_jump_chord_focuses_the_last_notified_pane` and `settings_keys_section_scrolls_to_every_row`).

#### Sounds

`[client] sound = true` (herdr's `[ui.sound]`) plays a sound on agent state changes, on its own channel: it does not depend on `notify-delivery` (sounds still play with delivery `off`) or on `notify-states`. Herdr's event mapping: an arrival in `blocked` or `question` plays the **blocked** sound wherever the agent is (input is needed even in view), and a finish (`done`, or `idle` straight from `working` / `blocked` / `question`) plays the **done** sound only for an agent outside the shown window. A repeat of the same state, the daemon's seen flip (`done` → `idle`), and the status refresh never sound. `sound-done` and `sound-blocked` name custom sound files (a relative path resolves from the config file's directory, `~/` from home); empty, or a file that does not exist, plays the built-in OS sound — `/System/Library/Sounds/Glass.aiff` (done) and `Ping.aiff` (blocked) on macOS, the freedesktop theme's `complete.oga` and `bell.oga` elsewhere on Unix. `sound-player` is the command the file is handed to, split on whitespace with the file appended as the last argument (e.g. `mpg123 -q` or `pw-play`); empty uses `afplay` on macOS and `paplay` elsewhere. The player runs argv-only (no shell), detached, with a reaper thread, so the pump never waits on it. When no sound file or player is available (Windows has neither built in) the terminal bell (BEL) rings with the next frame instead; the `inside` toast's own bell stands in when one is up. `sound-mute` lists agent labels that never sound (case-insensitive), herdr's `[ui.sound.agents] <agent> = "off"`. All five keys sit in the settings dialog's `notify` tab and reload live (pinned by `transitions_map_to_herdr_sounds`, `per_agent_mute_filters_sounds`, `plan_prefers_custom_then_builtin_then_bell`, `sound_keys_resolve_defaults_and_explicit_values`, `transition_sounds_play_per_event_and_respect_the_mute_list`, `notify_section_lists_the_sound_keys` and `settings_sound_rows_write_and_apply_live`).

Departures from herdr, by decision: sounds default **off** (herdr defaults on) so an upgrade stays silent and no test or headless attach makes noise; the built-ins are the OS's stock sounds rather than bundled mp3s, keeping audio assets out of the crate; the per-agent setting is a mute list rather than herdr's `default` / `on` / `off` per agent (herdr's `on` behaves exactly as an unlisted agent, so nothing is lost), and droid is not muted by default; there is no Windows player (herdr drives PowerShell's MediaPlayer), the bell stands in; and every attached render client with `sound` on plays the sound (herdr sends it to the foreground client only), so two attach windows on one machine both sound. Ad-hoc `show-notification` stays silent, as herdr's `notification show` is by default; a per-call sound option and a pane-bell sound are not wired.

#### Ad-hoc notifications

`show-notification [-b body] [--] <title>` (herdr's `notification show`) lets a script, hook, or embedder raise a notification through the same pipeline agent transitions use: the daemon broadcasts `%notification` and every attached render client delivers it through its own `[client] notify-delivery` channel — the `inside` toast (the title bold, the body dim, a click only dismisses it since no pane raised it, plus the bell), the `terminal` OSC 9/99 escape, or the `system` notifier — with the script's title in place of `par-mux`. `off` drops it. Unlike an agent transition it ignores `notify-states` and the background-only rule (a script asked for it explicitly), and it never moves the notify-jump target. Each client rate-limits its deliveries to ONE PER SECOND (herdr's `API_NOTIFICATION_RATE_LIMIT`): a second inside the window is dropped with a status flash — a script that loops `show-notification` cannot spawn an osascript per call — and the `inside` delivery also refuses while one of its toasts is still up (herdr's `Busy`). The title is the rest of the line (one quoted word is unquoted) and is required; control characters fold to spaces, and the title and body truncate at 80 and 240 characters. With no render client attached nothing is delivered; the reply is empty either way. From a shell: `par-mux --cmd "show-notification -b 'see the log' 'build failed'"` (pinned by `parses_show_notification`, `notification_round_trips_any_text`, `adhoc_notification_delivers_through_the_channel_without_a_target` and `adhoc_notifications_are_rate_limited_to_one_per_second`).

#### Herdr-parity display options

`[client] pane-gaps` (cells, default 0) insets every pane's rect per side so theme-background gap bands separate the panes (content, borders, labels, and the cursor mapping all follow the inset; the shared dividers keep their geometry and stay drag handles); `[client] scrollbar-gutter` (default off) reserves the right-edge column of every pane rect — content narrows by one cell and the gutter shows a minimal `▐` scroll-position indicator; without the reserved gutter nothing paints — the cue is suppressed rather than land outside the pane, and the boundary divider stays continuous; `[client] drag-cursor-shape` (default off) shapes the resize feedback while the pointer is over a resize handle — a pane divider, the side panel's edge column (the width drag), or its rule row (the areas' split drag) — on hover or during a live drag (a sidebar drag keeps its shape with the pointer off the panel too): the POINTER carries the axis-correct resize shape via OSC 22 (`ew-resize`/`ns-resize` — the escape ghostty, kitty, and xterm accept; the axis-named `col-resize`/`row-resize` spellings are NOT used because ghostty's macOS backend silently drops them while kitty accepts both). Each resize shape is immediately followed by its X11 cursor-font synonym (`sb_h_double_arrow`/`sb_v_double_arrow`) because iTerm2's OSC 22 map knows only X11 names and RESETS its pointer on any name it does not know — one write per vocabulary, each terminal applying the one it knows. A terminal without OSC 22 keeps its own pointer; a par-term host gains the same cue once its frontend consumes the core's `pointer_shape_changed` event (core parser shipped 2026-10-16, filed upstream) and, during a live drag only, the host's text cursor becomes a best-effort DECSCUSR steady block; leaving the handle, drag end, and detach all restore.
`[client] show-label-in-border` (default ON — pane titles show in their borders; prefix `l` toggles) embeds each pane's title in its top border edge in the per-pane-box modes.
All rebind on the reload chord and gen-config emits them. The per-pane chrome these options paint (the border ring of the per-pane-box modes, the gap band, the gutter column) is declared on every size report (`refresh-client -C … -I`), so the daemon sizes each pane's PTY to the painted interior and a full-screen program's last row and column are never cropped under the ring; a runtime change (the border-style chord, a reload) re-declares and re-fits.

#### Session/window switching

Prefix `o` and the arrows cycle panes; prefix `n`/`p` move to the next/previous window of the shown session; prefix `(`/`)` move to the previous/next session — every switch is the passthrough dispatch semantics (daemon-side select, then the renderer re-seeds from a fresh layout report and per-pane replays; the status bar re-queries for the new view).
Workspace switching rides the same contract: prefix `W`/`C-w` send `select-workspace -t +N` (next/previous in id order) and the renderer re-seeds onto the workspace's session's active window; a workspace with no sessions moves the selection only, and the workspaces segment's active marker moves via the `%workspaces-changed` re-query.
Settable as `[client] workspace-next` / `workspace-prev` (defaults `W`, `C-w`; prefix `w` opens the session/window picker).

#### Cursor

Each frame places the host cursor at the FOCUSED pane's tracked cell — the pane emulator's cursor position mapped through the pane's rect origin and the client scroll offset — with the pane's tracked DECSCUSR shape (`CSI Ps SP q`) and DECTCEM visibility honored: a pane that hid its cursor (`CSI ?25l`) keeps it hidden, and a scrolled-off-live view (wheel or prefix-[) hides the host cursor entirely, since the live cell is not on screen. A flushed frame always repositions (the diff's per-cell CUPs moved the host cursor), and so does any change in the placement the client believes it last wrote. A quiet pump also re-asserts the placement once per `CURSOR_HEARTBEAT` (1 s) while a pane cursor is shown (not hidden, not scrolled off live, no modal up). The write is idempotent on a healthy host (a few bytes per second), and it heals a host terminal that lost the cursor state outside the client's guarded writers within a second (pinned by `heartbeat_re_emits_the_placement_when_quiet`). Set `PAR_MUX_CURSOR_TRACE=1` on the attach client to log every placement write, and every reason the focused cursor cannot be placed, to stderr (no focused pane, no layout rect, no emulator, view scrolled, DECTCEM hide, cell outside the content view). In render mode stderr is usually the tty being drawn, so redirect it: `PAR_MUX_CURSOR_TRACE=1 par-mux attach 2>/tmp/cursor.log`. See [TROUBLESHOOTING.md](TROUBLESHOOTING.md#the-pane-cursor-disappears-in-render-mode).

#### Focus indicator

Dividers follow the tmux half rule. On a divider shared by two panes, the half nearer the focused pane uses `[client] border-active-color` (default bright cyan, indexed 14, bold) and the other half uses `[client] border-color` (default dim).
A vertical divider highlights its top half while the left pane is focused and its bottom half while the right pane is; a horizontal divider highlights its left half for the top pane and its right half for the bottom pane.
A divider that does not touch the focused pane is plain throughout, and a divider being dragged renders reversed.

#### Background fill

Every frame cell carries the host terminal's resolved background before painting, so unwritten cells (rows below the layout, short history lines, wide-char spacers) render as background instead of the terminal default (the light-grey bands a dark-theme host showed).
The color comes from an OSC 11 probe (`ESC ] 11 ; ? ST`, 150 ms deadline, poll(2)-based) run right after raw mode comes up and before the pump's stdin reader starts; when the probe fails (Windows and non-xtermish hosts) the fill stays at the terminal default, and a corrected `set-client-colors` rides the probe so the daemon's theme record follows the host. The same window also probes the cell pixel size with XTWINOPS `CSI 16 t` and, on a reply (`CSI 6 ; height ; width t`), follows the handshake's `refresh-client -C … -p 10x20` construction default with the measured `WxH` — the daemon's cell-size record is latest-report-wins, so panes re-fit to the real cell metrics; a silent host keeps the default.

#### Themes

`[client] theme` picks the render-mode color theme from herdr's built-in presets: `default` (the built-in look above, unchanged), `auto`, or one of `catppuccin`, `catppuccin-latte`, `tokyo-night`, `tokyo-night-day`, `dracula`, `nord`, `gruvbox`, `gruvbox-light`, `one-dark`, `one-light`, `solarized`, `solarized-light`, `kanagawa`, `kanagawa-lotus`, `rose-pine`, `rose-pine-dawn`, `vesper`. herdr's aliases such as `tokyonight`, `latte`, `dawn`, and `light` work too, and herdr's 16-color `terminal` reads as `default`; an unknown name warns on the status row and uses `default`.
A preset recolors the chrome: pane rings and dividers (accent focused, overlay0 unfocused), the tab strip (panel bar, surface0 inactive blocks, accent active block), the status row (text on surface0, the shaded block on the accent), the side panel's active band and agent-state glyphs, toast dots, and overlay rings. Pane content keeps its own colors, and an explicit `border-active-color` / `border-color` still wins over the theme.
`theme = "auto"` follows the host's light/dark appearance: `theme-dark` (default `default`) on a dark host, `theme-light` (default `catppuccin-latte`) on a light one. The appearance comes from the launch OSC 11 background probe (herdr's rule: Rec. 601 luma at or above half scale is light), from a late OSC 11 reply the stdin parser consumes, and from mode 2031: the render client enables `CSI ? 2031 h` (and disables it on exit), and a host that supports it reports each switch as `CSI ? 997 ; 1|2 n`, which outranks background inference and re-queries OSC 11 so the fill follows. A host that answers neither (Windows consoles, non-xtermish terminals) counts as dark, so `auto` shows `theme-dark` there.
All three keys are live on reload and editable in the settings dialog's `borders` tab (pinned by `theme_presets_and_auto_follow_the_host`, `auto_picks_from_the_host_appearance`, `color_scheme_reports_reach_the_theme_not_the_pane`, and `settings_theme_rows_write_and_apply_live`).

#### Kitty graphics

`[client] kitty-graphics` (default on; herdr's `terminal.kitty_graphics`) draws the kitty graphics protocol images panes emit on the host terminal — images the pane sends as direct data (`t=d`) or through kitty's temp-file medium (`t=t`; the `t=f` file and `t=s` shared-memory media are refused by the emulator's default file-media policy, so a sender that picks them renders nothing). Each pane's emulator already parses the pane's kitty APC sequences into RGBA placements; ratatui cells cannot carry them, so every frame the render client re-emits each visible placement as its own kitty commands: one upload per distinct image (`a=t`, RGBA `f=32`, direct data in 4096-byte base64 chunks) and a placement (`a=p`) at the host cell the image occupies, with `c=`/`r=` stating the cells the pane's grid reserved for it, cropped (`x=`/`y=`/`w=`/`h=`) to the pane's content view and the client scroll offset. Every command carries `q=2` (the host never answers into the client's stdin) and placements carry `C=1` (the host cursor never moves, so an image on the bottom row cannot scroll the frame). Uploads and placements diff against what the host already holds, so a quiet frame writes nothing; a moved image re-places without re-uploading, and a gone one is deleted. A full repaint (seed, host resize, zoom, window switch) first deletes everything the client put on the host — hosts differ on whether a screen clear removes kitty state — and the next frame redraws what is still wanted. Placements hide while a modal overlay or popup covers the panes (an image would paint over it) and are left out where the in-app toast overlaps them; detach and turning the key off delete everything the client put on the host.

Fallback: the launch probe (the same 150 ms raw-mode window as the OSC 11 / cell-size probe) asks the host with a kitty query (`ESC _ G i=31,s=1,v=1,a=q,t=d,f=24;AAAA ST`) followed by DA1 (`CSI c`) as the ordering sentinel — kitty's documented detection: a kitty-capable host answers `ESC _ G i=31;OK ST` before its DA1 reply, any other host answers DA1 alone. Nothing is ever drawn unless the host answered `OK`; a late answer is consumed by the stdin parser (never forwarded to a pane) and enables graphics from the next frame. herdr gates on its config key alone; par-mux adds this host check so a host without the protocol never receives image bytes — only the one query, which the seed's screen clear wipes if such a host printed it. With the key off the query is not sent at all. Windows hosts skip the probe and stay off.

Image geometry follows the daemon: the render client sets every pane emulator's cell pixel size to the one it reported (`refresh-client -p`: 10x20 until the probe's XTWINOPS reply, the probed size after), so an image without `c=`/`r=` advances the mirror's cursor by the same rows it advanced the daemon's pane. A replay (`refresh-client -t`: seed, host resize, zoom, window switch) rebuilds the grid from the daemon's screen state, which carries no images, so a pane's images are dropped there rather than redrawn at stale rows; an image emitted before the client attached, or before such a refit, is not shown until the pane draws it again. Scope: kitty placements at the cursor (`a=T`/`a=p`) only — Unicode-placeholder virtual placements (`U=1`), parent-relative placements, images in scrollback, and Sixel / iTerm2 images are not drawn (pinned by `placement_maps_through_the_layout_and_sidebar`, `host_bytes_upload_once_place_and_diff`, `session_gates_on_support_and_config_and_hides_under_modals`, `replay_drops_the_mirrors_graphics`, `kitty_query_reply_and_da1_sentinel_strip_in_order`, `late_kitty_reply_reports_and_never_reaches_the_pane`, `invalidate_deletes_what_the_host_holds_then_redraws` and, PTY-level against the real binary, `render_mode_kitty_graphics_reach_a_kitty_host_positioned`, `render_mode_kitty_graphics_stay_off_a_silent_host`, `render_mode_kitty_graphics_config_off_sends_nothing`). The key is live on reload and a toggle in the settings dialog's `general` tab.

#### Popups

Popup command windows (herdr's `type = "popup"` custom commands): a `[[client.popup]]` table binds an after-prefix key to a command that runs in a floating, bordered box over the panes without touching the tiled layout.

```toml
[[client.popup]]
key = "t"              # the key after the prefix (tmux spelling: t, C-g)
command = "lazygit"    # shell text, one line (run through /bin/sh on unix)
width = "80%"          # outer width: cells (an integer) or "N%" of the pane area
height = "80%"         # outer height, same spelling; both default to half the area
description = "git"    # optional: the box's border title (default: the command)
```

Prefix + the key opens the popup. The size is the OUTER box, border included, centered over the pane area (right of the side panel), clamped to that area with a 6x4 minimum. A string size must be a percentage (`"120"` is refused; write `120` for cells), as in herdr. Only one popup is up at a time. An area under the minimum refuses with `popup: the pane area is too small`.

The command runs on the daemon. The client opens a scratch window named `popup` beside the shown window (par-mux's `new-window` never selects it) and mirrors its one pane into the box over a [`control-pane`](#raw-pane-streams) stream on a second connection. The stream seeds the box atomically, and its size report sizes the scratch window to the box's inner grid, so the command sees the box's columns and rows and follows host resizes and side-panel toggles. The command runs as written through the pane shell — the parser's start-command cut stops `new-window`'s flags at the `--`, so an inline `-c`/`-t`/`-n` is no longer misread and the launcher-script workaround is gone.

While the popup is up it owns all input, as herdr's session-modal popup does: typed bytes, pastes (framed by the popup's own bracketed-paste mode) and keys (re-encoded against the popup's own input modes, so DECCKM arrows reach a TUI) go to its pane. The host cursor sits in the box and mouse reports are consumed. The prefix stays live:

- prefix + the popup's key closes it;
- prefix prefix sends the literal prefix;
- prefix `d` closes it and detaches;
- every other chord is consumed.

The popup also closes when its command exits. Closing kills the scratch window and the tiled view takes input again. herdr has no close key (a popup closes only on exit or its `popup.close` API), so the toggle key is a par-mux addition.

Known deviations:

- The command starts in the daemon's working directory, not the focused pane's.
- The scratch window appears in OTHER clients' tab strips while the popup is up. This client's strip hides it.
- A client killed without its exit path (SIGKILL) leaves the scratch window running. Close it with `kill-window`.

The popups are editable in the settings dialog's `general` tab: one `popup` row per entry spelled `KEY [WxH] COMMAND` (`t 80%x20 lazygit`, a missing axis as `-`), plus a `+ add popup` row. Enter opens the prompt. A bad spelling or a key another popup holds flashes and writes nothing, and an empty commit deletes the popup (written as `popup = []`). A popup key that a built-in chord already owns never fires and warns at launch and on reload. An invalid entry (no key or command, a multi-line command, a duplicate key, a size like `"120"`) never makes the file unreadable, so the daemon's own settings still load. The attach client refuses it the way it refuses a bad chord: launch fails with the reason, and a reload flashes `reload failed: …` and keeps the live popups. Pinned by `popup_size_parses_cells_and_percentages`, `popup_tables_deserialize_and_round_trip`, `geometry_centers_clamps_and_floors`, `popup_key_opens_a_scratch_window_and_mirrors_its_stream`, `popup_captures_input_and_its_key_closes_it`, `popup_closes_when_its_command_exits_and_on_detach`, `settings_popup_rows_add_edit_and_delete`, and the end-to-end `render_mode_popup_runs_a_command_in_a_floating_box_and_closes`.

#### Bracketed paste

A host in bracketed-paste mode (`ESC[200~` … `ESC[201~` around every paste) has its paste bodies treated as an opaque byte run in render mode: the body never meets the prefix scan or the chord table (a pending prefix is cancelled), and it forwards to the focused pane — embedded prefix bytes and escape fragments included, since it is text the user meant to insert, not keystrokes to decode.
The host's markers are consumed; when the focused pane has bracketed paste on (DECSET 2004), the body is re-framed with the pane's own `ESC[200~`/`ESC[201~` after any embedded terminator is stripped.
A modal that owns the keyboard (rename prompt, menu, help, picker) receives the body as typed text, and the scroll viewport consumes it.
A paste longer than 1 MiB streams out in chunks and stays in paste mode until its terminator, so no byte of it reaches the chord scanner.
Passthrough mode tracks the paste markers and skips the prefix scan inside a paste; the bytes themselves flow through untouched.

#### Input methods (IME / CJK)

The host terminal owns IME composition (preedit and the candidate window) for every TUI, so the attach client only ever reads committed text as UTF-8 bytes on stdin; it does not use crossterm's event reader, and no terminal-side protocol reports preedit to a TUI. What the client controls:

- **CJK text forwards verbatim.** Committed text typed without the prefix reaches the focused pane byte for byte.
- **A non-ASCII key after the prefix is swallowed whole** (render and passthrough). An IME left in a CJK mode makes prefix chords commit as non-ASCII characters; the chord table is ASCII, so the client consumes every byte of the character (split across reads too) instead of forwarding a torn UTF-8 tail, and flashes `prefix X: chords are ASCII keys (switch the input method to ASCII)`. A fullwidth form (U+FF01..U+FF5E, what a CJK IME in fullwidth mode commits without preedit) folds to its ASCII chord, so prefix `ｄ` detaches and prefix `％` splits.
- **Modal text fields take CJK.** The rename/create prompt, the picker and help filters, and the scroll-mode search reassemble multi-byte characters; a malformed byte never enters a field. A CJK window or pane name commits intact.
- **Alt on a CJK character decodes whole.** An ESC-prefix Option key over a non-Latin layout (`ESC` + a multi-byte character) is one Alt key on the character, re-encoded for the pane; no continuation byte leaks as plain input.
- **The tab strip, status rows, and overlay rows lay out by display width.** They measure text in cells, so a wide glyph takes two columns and is never split at a truncation edge. The side panel's names, pane border labels, and toast titles still count characters.
- **Hidden-cursor IME anchor (render mode).** When the focused pane hides its cursor (`CSI ?25l`, as agent CLIs that paint their own cursor do), the frame still parks the hidden host cursor on the pane's tracked cell after hiding it, because the host terminal opens preedit and the candidate window at its cursor even when hidden. Without it the candidate window opened wherever the last diff cell was written. A scrolled-off-live view or an open modal parks nothing. Passthrough already ends every status draw on the tracked cell.

Not ported from herdr, with the reason:

- **Switching the host input source to ASCII while the prefix is armed** (herdr `experimental.switch_ascii_input_source_in_prefix`). It needs per-OS FFI (Carbon `TISSelectInputSource` on macOS, a Korean-IME-only toggle on Windows, nothing on Linux) and acts on the machine the client runs on, which is the wrong machine when `par-mux attach` runs over ssh. The swallow-and-hint rule plus the fullwidth fold cover the failure it prevents.
- **Revealing a hidden pane cursor for IME** (herdr `experimental.reveal_hidden_cursor_for_cjk_ime`, with `cjk_ime_agents` and `cjk_ime_cursor_shape`). It shows an extra hardware cursor next to the TUI's own painted cursor. The hidden anchor above gives macOS candidate-window tracking without that cost; the visible-reveal mode stays a follow-up.

#### Limitations

Forward-only mouse re-encoding (the pane's negotiated dialect — SGR, or classic/urxvt when 1006 was never enabled) and wheel scrollback is view-only (no pane resize probe).
A host resize re-fits the render client end to end (the new grid is reported with the content height, the daemon re-divides, panes re-seed, the status row repaints at the new bottom row — pinned by `render_mode_host_resize_refits_window_layout_and_status_row`), and a daemon-side zoom (`resize-pane -Z`) re-renders the visible layout — the zoomed pane alone — with `-Z` again restoring the split (`render_mode_daemon_zoom_shows_single_pane_and_unzoom_restores_split`); both in `crates/par-mux/tests/mux_attach.rs`. [MANUAL-PASS.md](MANUAL-PASS.md) carries the human checklist for Ghostty / iTerm2 / Terminal.app / SSH.

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
mode = "render"       # default attach mode: render (built-in default) | passthrough
reload = "C-b C-r"    # the chord that reloads this file mid-attach
split-right = "%"     # split the focused pane right
split-down = "\""    # split the focused pane down
kill-pane = "x"       # kill the focused pane
break-pane = "!"      # break the focused pane out to a new window
join-pane = "J"       # window picker in join mode (render mode)
palette = ":"         # the filterable command palette (render mode)
new-window = "c"      # new window in the focused pane's session
swap-prev = "{"       # swap the focused pane with the previous pane
swap-next = "}"       # swap the focused pane with the next pane
move-window-left = "<"  # move the active tab left in the tab order
move-window-right = ">"  # move the active tab right in the tab order
tab-strip-left = "H"  # scroll the tab strip left (render mode)
tab-strip-right = "L"  # scroll the tab strip right (render mode)
workspace-next = "W"  # select the next workspace
workspace-prev = "C-w"  # select the previous workspace
resize = "R"          # enter the sticky resize mode (arrows move the edge)
resize-step = 1       # cells per resize step (arrows and divider drag)
help = "?"            # the bindings help panel
picker = "w"          # the session/window picker modal
workspace-picker = "g"  # the workspace list picker modal (render mode)
sidebar = "s"         # toggle the workspace side panel (render mode)
sidebar-width = 20    # the side panel's columns when shown (6-60)
sidebar-on-launch = true  # show the side panel when a render client attaches (prefix s still toggles it)
agent-rows = [["state_icon", "workspace", "project"], ["agent"]]  # side-panel agent entry lines (tokens: state_icon state workspace project message agent pane)
agents-sort = "grouped"  # agents area ordering: grouped | priority (the header toggle's setting)
notify-delivery = "terminal"  # agent state notifications: off, inside (a toast over the panes + the terminal bell), terminal (OSC 9/99), system (osascript/notify-send)
notify-states = ["blocked", "done"]  # the agent states whose arrival notifies (working, blocked, question, done, idle)
# git-poll-secs = 10                     # seconds between sidebar git-badge polls (0 disables; else 3-600, default 10)
sound = false         # agent state sounds: blocked (needs input) and done (finished out of view)
sound-player = ""     # player command, the file appended (empty = afplay on macOS, paplay elsewhere)
sound-done = ""       # done sound file (empty = the built-in OS sound; relative to this file)
sound-blocked = ""    # blocked sound file (empty = the built-in OS sound; relative to this file)
sound-mute = []       # agent labels that never sound, e.g. ["droid"]
notify-jump = "j"     # focus the pane behind the last agent notification (render mode)
status-bar = "S"      # toggle the bottom status bar (render mode; shown by default)
respawn = "r"         # respawn the focused pane when it is held dead (both modes)
zoom = "z"            # zoom the focused pane (render mode; the daemon's resize-pane -Z)
rename-window = ","   # rename the shown window via the prompt (render mode)
rename-pane = "$"     # rename the focused pane via the prompt (render mode)
border-cycle = "B"    # cycle the border styles (render mode)
label-toggle = "l"    # toggle pane titles in the borders (render mode)
border-lines = "herdr"    # render mode border styles: herdr | unicode | double | heavy | ascii
pane-borders = false       # render mode: full ring per pane (focused accent) instead of shared dividers
                           # (the ring and its label take border-active-color / border-color below)
show-label-in-border = true   # with pane-borders: the pane's title embedded in its top edge
pane-gaps = 0                 # render mode: theme-bg gap bands between panes (cells per side)
scrollbar-gutter = false      # render mode: reserve a right-edge gutter column (scroll indicator)
drag-cursor-shape = false     # render mode: resize pointer over dividers (OSC 22) + drag cursor shape
kitty-graphics = true         # render mode: draw panes' kitty images on a kitty-capable host (see Kitty graphics)
session-picker = false        # target-less attach with several sessions: pick one inline instead of the refusal (terminals only)
border-active-color = ""      # render mode: focused divider half, pane ring and label, #rrggbb (default bright cyan, bold)
border-color = ""             # render mode: unfocused dividers, pane rings and labels, #rrggbb (default dim)
theme = "default"             # render mode color theme: default | auto | a herdr preset (see Themes)
theme-dark = "default"        # the preset theme = "auto" uses on a dark host
theme-light = "catppuccin-latte"   # the preset theme = "auto" uses on a light host

# Popup command windows (render mode; see Popups): prefix + key runs the
# command in a floating box over the panes.
# [[client.popup]]
# key = "t"
# command = "lazygit"
# width = "80%"   # cells (integer) or "N%"; default half the pane area
# height = "80%"

[daemon]
socket = "default"    # named default socket, or an absolute path
state-dir = ""        # empty = the OS default state directory
pane-endpoints = false
expose-control-socket = false
remain-on-exit = false  # hold a pane whose child exited instead of removing it (the attach respawn cue; under the default auto-remove a shown pane's death flashes the affordance instead)
exit-empty = true       # exit after the 5 s grace once no sessions (or only dead panes) remain; attached clients do not hold it
split-start-dir = "home-dir"  # what a split-window pane starts in with no -c: "home-dir", "previous-pane-dir" (the target pane's live cwd), or "workspace-dir" (the workspace's launch folder)

[daemon.agent-append]
# Extra words agent-start appends when launching/resuming an agent — the
# resume flags a shell alias would carry, e.g.:
#   claude = "--continue --name {name}"
# {name} expands to the pane cwd's basename; unknown {tokens} stay
# literal. Words are whitespace-split and each is shell-quoted on typing,
# so a value can never run shell syntax. Every supported kind has a key;
# blank = launch bare.
claude = ""
codex = ""
grok = ""
muse = ""
droid = ""
copilot = ""
cursor = ""
kimi = ""
qwen = ""
qodercli = ""
devin = ""
gemini = ""
amp = ""
mastracode = ""
```

Every setting maps to an existing flag or resolution rule: `[client]` keys
feed `attach` (`--prefix`, `--mode` — a config `mode` applies only when
`--mode` is absent, the default is `render`, and an unknown file value
falls back to `render` with a warning on stderr); the
`[daemon] socket` accepts a NAME (as `par-mux <name>` spells it) or an
absolute path, and sits in the same precedence chain as the env/flag tiers
of `resolve_socket_path` (`$PAR_MUX_SOCKET` still beats the file; an
explicit `--socket`/positional still beats the env). Reload semantics per
setting:

| Setting | Reload behavior |
|---------|-----------------|
| `client.prefix` | **Live** — the running attach rebinds its prefix immediately (the old prefix stops intercepting the same keystroke). |
| `client.reload` | **Live** — it rebinds itself: the NEXT reload follows the new chord. The chord's key is matched by byte, so the default `C-r` (0x12) never collides with prefix `r` (respawn, the literal 0x72). |
| `client.split-right` / `split-down` / `kill-pane` / `break-pane` / `join-pane` / `palette` / `new-window` / `swap-prev` / `swap-next` / `move-window-left` / `move-window-right` / `tab-strip-left` / `tab-strip-right` / `workspace-next` / `workspace-prev` / `resize` / `help` / `picker` / `zoom` / `rename-window` / `rename-pane` / `border-cycle` / `label-toggle` / `workspace-picker` / `sidebar` / `status-bar` / `respawn` / `notify-jump` | **Live** — the running attach rebinds these chords immediately (parsed with the same prefix grammar as `prefix`; defaults `%`, `"`, `x`, `!`, `J`, `:`, `c`, `{`, `}`, `<`, `>`, `H`, `L`, `W`, `C-w`, `R`, `?`, `w`, `z`, `,`, `$`, `B`, `l`, `g`, `s`, `S`, `r`, `j`). |
| `client.border-lines` | **Live** — the border style re-applies on reload (`herdr`, `unicode`, `double`, `heavy`, `ascii`; an unknown value keeps the current set and says so on the status row). |
| `client.theme` / `theme-dark` / `theme-light` | **Live** — the theme re-applies on reload and `auto` re-picks from the last host appearance (an unknown name uses `default` and says so on the status row). |
| `client.resize-step` | **Live** — cells per resize step; a file value below 1 clamps to 1. |
| `client.popup` | **Live** — the `[[client.popup]]` tables re-apply on reload (an open popup stays up until it closes); `popup = []` clears them, an absent key keeps the live set, and an invalid entry fails the reload with the reason on the status row. |
| `client.mode` | Startup-only — changing it needs a re-attach. |
| `daemon.socket` | Startup-only by definition (the socket is bound at start). |
| `daemon.remain-on-exit` | **Live** — a `reload-config` whose file states a different value applies it at once and reports `applied: daemon.remain-on-exit`; the NEXT observed death honors it (the reaper reads the flag at every pass). |
| `daemon.exit-empty` | **Live** — a stated difference applies at once (`applied: daemon.exit-empty`); the accept loop re-reads the applied copy every idle tick, so an off daemon holds past the grace without a restart. |
| `daemon.split-start-dir` | **Live** — a stated difference applies at once (`applied: daemon.split-start-dir`); the next `split-window` honors it. A value other than `home-dir`, `previous-pane-dir`, or `workspace-dir` keeps the home default; a setting source that is gone (a stale pane cwd, a workspace without a launch folder) does too. An explicit `split-window -c` always wins. |
| `daemon.agent-append.<kind>` | **Live** — a stated difference applies at once (`applied: daemon.agent-append.<kind>`), one report line per supported kind; the next `agent-start` of that kind appends the words, and a daemon restart's resume invocation appends the same words onto the resume argv (deduped against the tokens the resume shape itself carries). Words are whitespace-split and shell-quoted on typing (a value can never execute shell syntax); `{name}` expands to the pane cwd's basename, unknown `{tokens}` stay literal. Blank = launch bare. |
| `daemon.pane-endpoints` / `daemon.expose-control-socket` | Startup-only in v1 (reported as restart-required). |
| `daemon.state-dir` | Startup-only — the daemon resolves the persist path once at startup and the persist worker writes to that fixed path. |

There are three reload entry points:

- **Control command `reload-config`** (no arguments — it always re-reads
  the canonical path): the daemon re-reads the file and replies one line
  per `[daemon]` setting, `unchanged: <name>` or
  `restart-required: <name>` — except `daemon.remain-on-exit`,
  `daemon.exit-empty`, `daemon.split-start-dir`, and every
  `daemon.agent-append.<kind>` key, the live settings: a stated
  difference is applied at once and reported
  `applied: daemon.remain-on-exit` / `applied: daemon.exit-empty` /
  `applied: daemon.split-start-dir` / `applied: daemon.agent-append.<kind>`. The file speaks only for settings it
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
- **Propagation to every attached client**: a successful `reload-config`
  broadcasts `%config-changed`, so the settings dialog's write (which
  runs the reload chord's path) reaches every OTHER attached render
  client from the same host: each re-reads the file and re-applies its
  `[client]` settings — chords, notify delivery and states, border and
  display options, agent rows — through the same rebind its own reload
  chord runs, flashing `config re-applied`. The writing client already
  applied the file, so its own copy of the broadcast is a no-op; a
  follower never sends `reload-config` back. The scope line: a bare
  edit of the file reaches no client until SOME client reloads (any
  reload — one chord press, one menu row, or one control-client
  `reload-config` — then reaches all of them; there is no file watcher).
  Passthrough clients ignore the cue (they re-read on their own chord).
  A changed `sidebar-width` applies through the same geometry/refit path
  as the dialog and persists the new width. Unrelated reloads preserve
  each client's dragged width.
- **`par-mux --gen-config`** writes the config file with the CURRENT
  EFFECTIVE values (file if present > env > built-in defaults; startup
  CLI flags are not introspectable after parse, so they do not appear —
  except where the generating invocation's flag tier is visible to the
  resolution, i.e. the socket and the daemon bools of the generating
  run). Never overwrites an existing file without `--force`. The
  location follows `$PAR_MUX_CONFIG` / the platform config dir.

`reload-config` mutates nothing except `daemon.remain-on-exit` and
`daemon.exit-empty` — it is
otherwise the report of what WOULD change plus the client-side live
rebinds above; a restart (`par-mux --restart`) applies restart-required
daemon settings.

| Item | Unix | Windows |
|------|------|---------|
| Transport | Unix domain socket at `<base>/par-mux-<name>.sock` | Named pipe; a marker file is written at the same path so staleness checks work uniformly |
| Default socket base | `$XDG_RUNTIME_DIR`, falling back to `<tmp>/par-mux-<uid>/` | Per-user temp dir |
| Access control | Socket file mode `0600` (owner only); the fallback directory is mode `0700` and its owner and mode are verified before bind and connect; accepted connections are refused unless the peer runs as the daemon's user | Owner-only security descriptor (system + creating user, nothing for anyone else) |

The fallback-directory guard is tmux's `/tmp/tmux-<uid>` defense: `/tmp` is world-writable and shared by every local user, so a socket named directly under it could be pre-bound by another user, whose server would then receive a client's keystrokes and clipboard. A fallback directory that exists but is owned by another user, grants group/other access, or is not a directory fails closed with `PermissionDenied` on both the bind and the connect side; remove or fix the directory to proceed. Explicit `--socket` paths are not guarded — whoever named the path chose its location. `$XDG_RUNTIME_DIR` needs no guard: the OS provisions it per-user.

Which socket one `par-mux` invocation targets follows one precedence (`resolve_socket_path` in `crates/par-mux/src/mux/ipc.rs`): an explicit `--socket PATH`, then the positional `NAME`, then `$PAR_MUX_SOCKET`, then the unnamed `default`. The env tier is what makes the client flags work from inside a pane — every pane spawns with `PAR_MUX_SOCKET` naming its own daemon, so `par-mux --cmd list-sessions`, `--stop`, and `--restart` typed there target that daemon instead of failing against `par-mux-default.sock`. An empty value counts as unset. Note that `--stop`/`--restart` resolved this way kill the daemon that owns the very pane the command was typed into — the panes die with it, which is the point of a restart but worth knowing before typing it.

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

Each registered client has a bounded broadcast queue (4096 lines). A client that stops reading its socket long enough for its queue to fill — 4096 undelivered lines, worst case ~128 MiB when every line carries a full PTY read — is **evicted**: the daemon logs a warning naming the reason (queue full — stalled, or draining slower than the burst), flags the connection, and its own reader/writer threads tear it down within a fraction of a second, closing the socket the client observes — and, like any registered connection's end, broadcasts `%client-left` to the clients that remain. A wedged client is disconnected rather than allowed to grow the daemon without bound. A client that keeps draining loses nothing. An evicted client simply reconnects and re-queries; pane output it missed is not replayed to it.

Eviction is by **queue depth only** — intended, and deliberately unlike tmux, whose control clients are evicted by output *age* (300 s) while buffering without bound in between. The depth policy bounds daemon memory at ~128 MiB per client; its accepted consequence is that a burst whose backlog passes 4096 lines evicts even a *healthy* client that drains continuously but slower than the producer — the backlog, not the client, is what hit the cap. The client reconnects and re-queries. Pinned by `a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap` (`crates/par-mux/src/mux/server/tests.rs`); an age-based policy would replace that test with a drains-slowly-survives one.

An **aggregate bound** sits above the per-client depth (ARC-008): once the clients' summed queue counters cross 8192 lines, the push evicts the client holding the DEEPEST queue — the stalled one — while the live ones keep draining. Only a queue at a quarter of the per-client depth (1024 lines) can be the victim, so an aggregate that crosses the cap through many shallow queues is read as a healthy crowd, not a stall, and nobody is evicted. The counters are the fan-out's successful pushes minus the client writer's drained lines — an approximation of the channel length a stall bound can afford (reply lines are uncounted; a drain lags its push; a client that leaves stops counting). Crossing the bound takes a genuine multi-client stall. Pinned by `the_aggregate_cap_evicts_the_deepest_queue` and `the_aggregate_cap_spares_shallow_queues`.


## Agent-State Integrations

par-mux owns the integrations that report agent state into the daemon:
the claude, codex, grok, muse, droid, copilot, cursor, qwen, qodercli,
kimi, devin, and mastracode session hooks
(script assets written under the par-mux config directory and registered in
each agent own config — a merge that preserves the user entries, comments,
and formatting) and the pi/omp agent-state extensions (a file drop into each
agent extension directory).
The two extension assets are generated, not hand-written: both dialects
render from the single template in `scripts/gen_extensions.py` (the shared
transport and session-ref plumbing live there once, the dialect wiring as
knobs), and `make extensions-check` (part of checkall) fails when a
committed asset is not what the template renders — fix a shared defect in
the template, never in one fork (`make gen-extensions` re-renders; ARC-002).
They are embedded in the binary and driven by five one-shot flags, none of
which starts or contacts a daemon:

```text
par-mux --install-agent-hooks            Install the claude/codex/grok/muse/droid/copilot/cursor/qwen/qodercli/kimi/devin/mastracode session hooks
par-mux --uninstall-agent-hooks          Remove them (config entries first, then the assets)
par-mux --agent-hooks-status              One "<agent> installed|absent" line per agent, plus the asset's version currency when installed (v2, `v1 → v2 outdated`, `asset missing`)
par-mux --integration-status              The same grammar for EVERY bundled asset: hook, extension, and skill lines
par-mux --install-agent-extensions        Drop the pi and omp extension files
par-mux --uninstall-agent-extensions      Remove them
par-mux --heal-agent-hooks                Rewrite a registered hook asset that went missing
```

The hooks never slow an agent down: outside a par-mux pane the scripts
exit on their env guards before doing anything, inside one the single
socket write carries a 0.5 s timeout, and every failure is swallowed
silently — a session start never blocks or fails because of them. codex's
hook payload names the thread rather than the rollout, so both codex hook
assets derive the id they report (and resume with) from the transcript's
`rollout-<timestamp>-<uuid>.jsonl` filename when it parses and differs
from the payload id; the `CODEX_THREAD_ID` fork guard still rules on the
payload id. The muse arm is the one that cannot carry `PAR_MUX_PANE_ID` at
all: muse clears its hook environment down to `MUSE_SESSION_ID`/
`MUSE_THREAD_ID` (verified live against 1.4.9/1.4.10), so its reporter
discovers the daemon sockets at the `default_socket_path` tiers and sends
`pane.report_agent_session_cwd` — the payload's cwd instead of a pane id.
The daemon resolves the pane itself, conservatively: local panes only, then
a pane already claimed by the agent, else an unclaimed pane whose spawn
command names the agent as a word, else any unclaimed pane in that
directory (the "typed `muse` inside a shell pane" host), ties to the lowest
pane id; a report with no match is an error reply the reporter ignores, so
fanning one report to every discovered socket — including daemons that own
no matching pane — is harmless. `muse resume` is workspace-scoped, which is
what makes even a wrong-pane tier-3 pick safe to restore.

The droid, copilot, cursor, qwen, qodercli, kimi, devin, and mastracode
arms are ports of herdr's integrations (Apache-2.0), each entries in the
agent's own config file whose reporter sends `pane.report_agent_session`
on session start:

| Agent | Config file (override env) | Entry |
| --- | --- | --- |
| droid (Factory) | `~/.factory/settings.json` (none) | nested `hooks.SessionStart` group, `{type, command, timeout}` |
| copilot (GitHub Copilot CLI) | `~/.copilot/settings.json` (`COPILOT_HOME`) | flat `hooks.SessionStart` entry, `{type, bash, timeoutSec}` (`powershell` on Windows) |
| cursor (`cursor-agent`) | `~/.cursor/hooks.json` (`CURSOR_CONFIG_DIR`) | bare `{command}` under camelCase `hooks.sessionStart`; a missing top-level `version` is added as `1` |
| qwen (Qwen Code) | `~/.qwen/settings.json` (`QWEN_HOME`) | nested `hooks.SessionStart` group, claude schema |
| qodercli (Qoder CLI) | `~/.qoder/settings.json` (`QODERCLI_CONFIG_DIR`) | nested `hooks.SessionStart` group, claude schema |
| kimi (Kimi Code) | `~/.kimi-code/config.toml` (`KIMI_CODE_HOME`) | a marked block of 12 `[[hooks]]` TOML tables appended after the user's content (rebuilt whole on install, cut on uninstall; two events carry `^AskUserQuestion$` tool matchers); commands run the `session`/`working`/`blocked`/`idle` actions |
| devin (Devin CLI) | `$XDG_CONFIG_HOME/devin/config.json` (else `~/.config/devin`; `%APPDATA%\devin` on Windows) | nested `hooks.<Event>` groups under six lifecycle events, every command the `session` action |
| mastracode (MastraCode) | `~/.mastracode/hooks.json` (none) | `{type, command, timeout(ms), description}` entries under event keys at the DOCUMENT root, one action per lifecycle event (11 events) |

Their hooks report identity (droid, copilot, and cursor plus herdr's resume
invocation as `session_resume_argv`: `droid --resume <id>`,
`copilot --resume=<id>`, `cursor-agent --resume <id>`); qwen, qodercli,
kimi, devin, and mastracode report identity with no resume argv (herdr's
reports for them carry none; qwen, qodercli, kimi, devin, and mastracode
originally shelled out to the herdr CLI — the ports speak the control
socket directly). kimi and mastracode hooks also report STATE: their
lifecycle actions map to `pane.report_agent` (`working`, `blocked`,
`idle`), so their panes are hook-authoritative and skip the scrape tier;
devin's reporter resolves its session id from the hook payload, else from
`devin list --format json` matched by working directory (never on the
fresh-turn events, where a stale session would be claimed). The remaining
agents get state from the scrape tier's bundled patterns below. `--install-agent-hooks` prints `skipped (… not installed)`
for any of these whose config directory is absent instead of failing
the run. par-term

invokes these flags for its integrations dialog, CLI, Settings status rows,
and startup self-heal instead of embedding its own copies, so the
integrations always match the shipped binary.

## Command Reference

The parser is deliberately minimal: whitespace-split with a per-command flag scan (each verb declares the flags that take a value; everything after a bare `--`, or the first non-flag word, is that command's own tail — the create commands' start commands read through exactly that cut). tmux's full argument grammar (per-command option tables, command sequences) is not implemented. Quoting is honored in a fixed set of places, all sharing one bounded grammar (single or double quotes, backslash escapes outside quotes, the `'\''` close-escape-reopen idiom, no interpolation): the `send-keys` payload and `-t` target, the `new-session -s NAME` / `new-window -n NAME` names, the `split-window -c DIR` / `new-window -c DIR` / `new-session -c DIR` start directories, the `select-pane -T TITLE` title, environment values (`new-session -e NAME=VALUE`, the `set-environment` name and value, the `rename-session` / `rename-workspace` names), the `set-buffer` payload, the `-t`/`-s` targets, and the `-F` format strings — so a name, directory, title, value, format, or target may contain spaces. The scan for a value-taking flag reads shell words and only matches an unquoted flag word, so the `-t` in `new-window -n 'a -t b' -t s1` is part of the name and the target is `s1`; a flag word given as another flag's value is that value, so `worktree-remove -t -f` targets a workspace named `-f` without forcing. Every other flag is whitespace-split, and `rename-window` takes the rest of the line after `-t`'s value as the name (whitespace-split tokens joined with single spaces — rename clients send the raw name, since quoting would become part of it; one leading `--` ends the flags and is dropped, so a name that itself starts with `--` needs one more `--` before it). `set-environment` reads its flags only until the first NAME word or `--`, so a VALUE spelled `-u` or `-t` is the value; `set-option`, `rename-session`, `rename-workspace`, and `set-workspace-respawn` read their `-t` the same way, so a key, name, or value spelled `-t` follows `--` (or is quoted). `join-pane` refuses any flag or word it does not read (tmux's `-l <size>` included — use `-p`). The three create commands (`new-session`, `new-window`, `split-window`) take a trailing start command, tmux's positional form: the raw text after the flags (or after a bare `--`) runs in the new pane instead of the default shell, the same shell handling `respawn-pane`'s command gets (a tail that is exactly one quoted word is unquoted before it runs, so `split-window -t %0 'sleep 60'` runs `sleep 60` — quoting groups like tmux's argv instead of fusing the command into one literal word); unknown value-taking flags on them are refused loudly rather than silently dropped (`new-session -F …` is an error, not a dropped format request, and tmux's `split-window -l <size>`/`-e`/`-F` are refused so their value never becomes the start command — use `-p` for the size). `new-window -e` is refused too: a pane's environment is its session's, which `respawn-pane` and restore re-seed from, so set it with `set-environment` or `new-session -e`. List replies have fixed shapes, and `-F` replaces them with per-row format expansions (see the table rows); an unknown `#{name}` expands to nothing, and text outside the braces rides verbatim.

A target placeholder in the table below (`<pane>`, `<window>`, `<session>`) is either the typed id (`%N`, `@N`, `$N`) or a **name**, resolved daemon-side against the tree:

- **`<pane>`** matches the pane's sticky user title (`select-pane -T`) exactly. The pane program's live OSC 0/2 title never matches — it changes with the running program and would make name targets flaky.
- **`<window>`** matches the window's name (`new-window -n`, `rename-window`) exactly, across every session.
- **`<session>`** matches the session's name (`new-session -s`) exactly. Session names are unique: `new-session`, `new-workspace` (its first session is named after the workspace), and `rename-session` refuse a name another live session already holds (`duplicate session: <name>`), and a nameless `new-session` takes the lowest unused numeric name (tmux's default) instead of a literal `0`. A duplicate from a legacy persisted tree still restores; targeting it stays the ambiguous-listing error.
- **`<workspace>`** matches the workspace's name (`new-workspace -n`) exactly, across every workspace. Workspaces are par-mux's own level above sessions (tmux has no concept): the hierarchy is daemon > workspace > session > window > pane, and the workspace id sigil is `+` (`+N`).

Ids always win over names: a value starting with the target kind's own sigil is the id, so a pane titled `%3` can never shadow pane `%3` (and a sigil-prefixed value that is not a valid id, like `%abc`, is a parse error). A name matching more than one pane/window/session is an **error listing the sorted candidate ids** (`ambiguous pane target: dup (matching: %0, %2)`) — never a silent pick, and the command acts on nothing. An unknown name errors (`no such pane: <name>`). One nuance: `send-keys` cuts its payload from the line with shell words too, so its `-t` honors the same quoting — a quoted target (`-t '%0'`, `-t 'my pane'`) rides whole where a bare one is a single whitespace token, and the payload is the raw remainder after the target word.

| Command | Arguments | Reply body | Broadcasts |
|---------|-----------|------------|------------|
| `new-session` | `[-s name] [-e NAME=VALUE]… [-t <workspace>] [-n window-name] [-c dir] [-x cols] [-y rows] [--] [command]` | The session id (`$N`) | `%window-add` per window, `%sessions-changed`; `%session-changed` to the issuer |
| `new-workspace` | `[-n name]` | The workspace id (`+N`); the new workspace becomes the daemon's active one and spawns its FIRST session (named after the workspace) with a window named `1` (the default shell), so it is landable the moment it exists | `%workspaces-changed`, `%sessions-changed` |
| `list-workspaces` | — | One `+N: name` line per workspace, id order; the daemon's active workspace ends with ` active` | — |
| `select-workspace` | `-t <workspace>` | empty; the workspace's previously active session resumes as the active session | `%workspaces-changed`, and `%client-session-changed` when the displayed session moved |
| `switch-client` | `[-t <session\|window\|pane>]` | Bare: the displayed session's id (`$N` — the active workspace's active session; empty when none). With `-t`: empty; the target's session becomes the displayed one (its workspace becomes active and it becomes that workspace's active session) and a window/pane target's window is selected. Persisted. Both attach modes (render and passthrough) send it on every user-initiated landing, so the follow broadcasts and a restored daemon all agree on what is shown | A target in the already-displayed session: exactly `select-window`'s (`%session-window-changed` when the active window moved, `%window-pane-changed`). A cross-session switch: `%workspaces-changed` when the workspace moved, `%client-session-changed` when the displayed window moved, `%window-pane-changed` for a window/pane target. Only `-t` is accepted — tmux's other `switch-client` flags are an error |
| `rename-workspace` | `-t <workspace> <name>` | empty | `%workspaces-changed` |
| `kill-workspace` | `-t <workspace> [-g]` | empty. A repo workspace with open linked worktree workspaces is refused (`… has linked worktree workspaces (+A, +B); use -g to close the group`) unless `-g`, which closes the whole group; checkouts on disk are untouched (see [Git worktree workspaces](#git-worktree-workspaces)). Feature token `group` | `%window-close` per killed window, then `%sessions-changed` and `%workspaces-changed` |
| `set-workspace-respawn` | `-t <workspace> [on\|off\|toggle]` | The flag's resulting state, `on` or `off`; the bare form is a read-only query. The keep-one-tab respawn option (see [Pane Reaping](#pane-reaping)); off by default, persisted | none |
| `new-window` | `[-t <session\|window\|pane>] [-n name] [-c dir] [--] [command]` | The window id (`@N`) | `%window-add` |
| `select-window` | `-t <window>` | empty | `%window-pane-changed`; plus `%session-window-changed` when the displayed session's active window moved |
| `kill-window` | `-t <window>` | empty | `%window-close`; `%sessions-changed` when it emptied the session |
| `rename-window` | `-t <window> [--] <name>` | empty | `%window-renamed` |
| `rename-session` | `-t <session> <name>` | empty | `%session-renamed` |
| `kill-session` | `-t <session>` | empty | `%window-close` per killed window, then `%sessions-changed` |
| `split-window` | `-t <pane\|window\|session> [-h\|-v] [-b] [-p 1-99] [-c dir] [--] [command]` | The new pane id (`%N`) | `%layout-change`, `%window-pane-changed` |
| `select-pane` | `-t <pane> [-T 'title']` | empty | `%layout-change`, `%window-pane-changed`; `%pane-title-changed` when `-T` changed the title |
| `pane-title` | `-t <pane>` | The pane's effective title as the body; an empty body (no lines) = no title set | — |
| `pane-info` | `-t %N [-M]` | One line, `%N @W COLSxROWS [cmd=<base64>] [exited=<code\|?>]` for the resolved pane: its window, current grid size (read-only; lets a mirroring client seed at the pane's size instead of resizing it), and — when knowable — the foreground command name (standard base64 of the basename of the deepest descendant of the pane's child process; absent on Windows and when the argv is unreadable, so older clients keep parsing the fixed prefix). The name is a hint set by the program itself (argv[0]); names with control characters or over 128 bytes are served absent. tmux's `#{pane_current_command}` equivalent, for close-confirmation prompts. `exited=` appears only for a held-dead pane: its exit code, or `?` when the code was unreadable (a dead pane has no `cmd=`, since its child is reaped) | — |
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
| `list-panes` | `[-t <window\|session\|pane>] [-F format]` | One `%N <leaf> <marker>` line per pane in layout leaf order (`leaf` is the deterministic index mapping tmux layout-string leaves to pane ids; `*` marks the window's active pane). `-t` scopes to one window — a session target means its active window, a pane target its owning window, a name a window then a session (every scope a client can name). `-F` replaces the row shape with one expansion per pane (`#{session_id}`, `#{session_name}`, `#{window_id}`, `#{window_name}`, `#{pane_id}`, `#{pane_active}`, `#{pane_leaf}`, `#{pane_title}`; an unknown name expands to nothing). Bare form: one `%N` line per pane, globally | — |
| `list-windows` | `[-t <session>]`, `-a`, `[-F format]` | One `@N <marker> <name>` line per window of that session in window order (`*` marks the session's active window; the name is the line remainder, so spaced names survive). `-a`: every session's rows, each prefixed with its session id (`$S @N <marker> <name>`), so a client finds a window's owner in one query (feature token `all`; not combinable with `-t`). `-F` replaces the row shape with one expansion per window (`#{session_id}`, `#{session_name}`, `#{window_id}`, `#{window_name}`, `#{window_active}`). Bare form: one `@N: name` line per window, globally | — |
| `list-sessions` | `[-t <workspace>] [-F format]` | One line per session, every workspace when bare (workspace order, then session order): `+W: wname: $N: name` — the session id is the last `$<digits>:` marker; `-t` restricts the listing to one workspace. `-F` replaces the row shape with one expansion per session (`#{workspace_id}`, `#{workspace_name}`, `#{session_id}`, `#{session_name}`) | — |
| `has-session` | `-t <session>` | empty when the session resolves; `%error` naming it otherwise — the exit-code probe agents script against (exit 0/1 through `--cmd`) | — |
| `list-agents` | — | Roster: one `%N <agent> <state> <source>` line per state-carrying pane (see below) | — |
| `list-commands` | `[-J]` | One line per dispatchable command, sorted: `name [feature …]`, plus a trailing daemon-level `features replay-held-state` line (see the capability-discovery bullet below). `-J` (feature token `schema`): the versioned JSON command schema instead (see the command-schema bullet below) | — |
| `client-snapshot` | `-t <window>` | One reply carrying the attach client's whole status read: `snapshot 1`, then `workspace`, `session`, `window`, `pane`, `agent` and `worktree` rows (see the client-snapshot bullet below) | — |
| `capture-pane` | `-t <pane> [-S start] [-E end] [-e]` | The captured lines | — |
| `wait-output` | `-t <pane> (-m TEXT \| -r REGEX) [-T ms] [-S start] [-E end]` | The first matching line once the pane's capture matches; `%error` on timeout, pane exit, or pane close (exit 0/1 through `--cmd`; see the wait-output bullet below) | — |
| `agent-start` | `-t <pane> [-T ms] [-x program] <kind> [arg …]` | `%N <agent> <state> <source>` once a hook claim or scrape rule detects the launched agent; `%error` on timeout, blocked startup, pane exit/close, or a busy pane (see the agent lifecycle bullet below) | `%agent-state-changed` (from the detection) |
| `agent-prompt` | `-t <pane> [-w] [-u state,…] [-T ms] [--] <text>` | empty; with `-w`, `%N <agent> <state> <source>` once the agent reaches an `-u` state, or `%error` on a stall or timeout | — |
| `agent-explain` | `-t <pane>` | `key value` lines: state, source (`hook`/`scrape`/`none`), hook reporter, scrape rule, pattern file, and the rule matching the live screen | — |
| `adopt-pane` | `-t <pane> <kind>` | `%N <kind> <state> scrape` from an immediate scrape classification of the live screen, else `%N <kind> unknown none`; `%error` on an already-labeled pane or a kind without scrape patterns (see the adopt-pane bullet below) | `%agent-state-changed` |
| `send-keys` | `-t <pane> <keys> [--enter]` | empty | — (space lives inside a quoted run; see below) |
| `refresh-client` | `-t <pane> [-C WxH] [-p WxH] [-I border=B,gap=N,gutter=G]` or `[-C WxH] [-p WxH] [-I …]` | Pane's screen-restore replay (with `-t`, no `-C`); empty otherwise | `%layout-change` (with `-C`) |
| `set-buffer` | `<content>` or `-H <hex bytes…>` (one byte per token, the `send-keys -H` form — the escape hatch for content a line-delimited wire cannot carry, newlines included) | empty | — |
| `set-client-colors` | `-f rrggbb` and/or `-b rrggbb` | empty | — |
| `set-environment` | `-t <session> NAME VALUE` or `-t <session> -u NAME` | empty | — |
| `show-buffer` | — | The buffer content | — |
| `paste-buffer` | `-t <pane>` | empty | — |
| `version` | — | The daemon's build stamp, one line: `<crate version>+<git sha[-dirty]>` (a `src-<16hex>` content digest when built outside a repository, e.g. from a crates.io tarball) | — |
| `show-notification` | `[-b body] [--] <title>` (the title is the rest of the line; `--` before a title that starts with `-`) | empty | `%notification` to every client (see [Ad-hoc notifications](#ad-hoc-notifications)) |
| `reload-config` | — | One line per `[daemon]` setting: `unchanged: <name>` or `restart-required: <name>` (live `applied: daemon.remain-on-exit`/`applied: daemon.exit-empty` when the file flips either), diffing the re-read config file against the daemon's applied settings (see [Configuration file](#configuration-file)) | `%config-changed` to every client when the file re-read succeeded (an `%error` reply broadcasts nothing) |
| `show-options` | — | One `key=value` line per applied daemon setting (booleans as `on`/`off`): `exit-empty`, `remain-on-exit`, `mode`, `prefix`, `reload`, `socket`, `state-dir`, `pane-endpoints`, `expose-control-socket` | — |
| `set-option` | `[-t <target>] <key> <value>` | empty. `-t` is accepted and ignored (every option is daemon-wide). The LIVE keys are `exit-empty` and `remain-on-exit` (`on`/`off`, also `true`/`false`); everything else is refused as restart-required. The change is volatile — the config file stays the durable record, and `reload-config` re-reads over it | — |
| `kill-server` | — | empty | `%exit` to every client, then the daemon exits |
| `worktree-create` | `-c <dir> -b <branch> [--base base] [-p path] [-s name]` (branch flag as in `git worktree add -b`, decision D7) | The new worktree workspace's id (`+N`). `git worktree add`s the branch (an existing local branch is checked out, otherwise it is created from `--base`, default `HEAD`) at `-p` or the default `<parent>/<repo>-worktrees/<branch-slug>`, opens the repo's workspace when none is open, then opens and focuses a workspace whose first pane starts in the checkout (see [Git worktree workspaces](#git-worktree-workspaces)) | `%workspaces-changed`, `%sessions-changed`, `%window-add` per opened workspace |
| `worktree-open` | `-c <checkout> [-s name]` | The workspace id (`+N`). A checkout (or any directory inside it) already open is focused (`select-workspace`); otherwise its workspace opens grouped under the repo's — the main checkout opens the repo workspace itself | As `select-workspace` when focusing; as `worktree-create` when opening |
| `worktree-remove` | `-t <workspace> [-f]` | empty. `git worktree remove`s the linked workspace's checkout, then closes the workspace. Refused without `-f` when the checkout has modified or untracked files or a pane carries an agent claim (any `list-agents` row, `idle` included); a repo workspace is refused (close it with `kill-workspace -g`). git runs first, so a refusal or git failure leaves the workspace open. Feature token `force` | `%window-close` per killed window, then `%sessions-changed` and `%workspaces-changed` |
| `list-worktrees` | — | One `+N <kind> <parent> <branch> <checkout>` line per worktree-backed workspace, id order: `kind` is `repo` or `worktree`, `parent` the open repo workspace a worktree is grouped under (`-` for a repo row or when none is open), `branch` `-` for a detached HEAD; the checkout path is the line remainder, so spaces survive | — |
| `observe-pane` | `-t <pane>` | `pane %N COLSxROWS [exited=<code\|?>]`, then the pane's screen-restore seed. The connection becomes a read-only raw stream of that pane: only its `%output`, `%pane-exited`, `%pane-respawned`, and `%exit` follow, and every later command is refused (see [Raw pane streams](#raw-pane-streams)) | — |
| `control-pane` | `-t <pane>` | As `observe-pane`, but the connection stays writable: `send-keys` and `refresh-client -t <pane> -C WxH` keep working on it (see [Raw pane streams](#raw-pane-streams)) | — |

Details worth knowing:

- **Workspaces are first-class** (daemon > workspace > session > window > pane). A session belongs to exactly one workspace for its whole life. A bare `new-session` lands in the daemon's ACTIVE workspace, creating a default workspace named `main` when none exists; `new-session -t <workspace>` names it explicitly (id or name — an unknown one fails the command). `new-workspace` creates and selects. Each workspace tracks its own active session; `select-workspace` resumes that session when it switches back. A workspace dies when its LAST session dies (`kill-workspace` kills the sessions outright; any kill cascade that empties a workspace removes it) — a workspace born empty (`new-workspace` before any session joins) is not removed until a session dies inside it. Empty workspaces do not keep the daemon alive (see [Shutdown Semantics](#shutdown-semantics)).
- **Bare `new-window`** targets the most-recently-created session (ids are monotonic). With `-t`, a session target appends the window to it; a window or pane target inserts the new window right after that window's position in its session. par-mux has no client-session attachment, so "newest" is the documented stand-in for tmux's attached-session resolution.
- **`new-session` and `new-window` take a trailing start command** in tmux's positional form, the create-command tail described above (`new-session -s logs -- tail -f x` runs `tail -f x` in the new pane). Unsupported tmux flags are refused loudly rather than silently dropped (`new-session` refuses `-F`/`-f`, `new-window` refuses `-F`/`-e`), so a refused flag's value never becomes the start command; tmux's other bare flags (`-d`, `-a`, …) are tolerated and ignored.
- **`version` exists for stale-daemon detection**: the daemon outlives its clients, so an old daemon silently serves new clients. A client compares the reply against its own linked core's `mux::build_stamp()`; differing stamps mean the daemon predates the client's build. Outside a repository both sides carry a content digest of the crate source, so same-version drift is still comparable; only a build with neither identity (`+unknown`) degrades to version-only comparison — a same-version mismatch is then unprovable and clients stay quiet rather than cry wolf.
- **`split-window` flags name the arrangement, not the divider**: `-h` puts the new pane beside the target, `-v`/default below it. `-p` is the percent of the split area given to the **new** pane (default 50; the target keeps the remainder). `-b` places the new pane **before** the target — left of it under `-h`, above it in the default direction — keeping exactly its `-p` share.
- **`split-window -c dir` and `new-window -c dir`** start the new pane in `dir` instead of the daemon-wide default. A `dir` that does not exist degrades to home rather than failing the command — the same rule a restore applies to a gone persisted cwd — and the new pane's screen says so (`par-mux: <dir> is gone; pane started in <home>`).
- **`resize-pane` relative form** moves the nearest divider of the pressed axis — an ancestor split of any depth, so a pane nested under cross-orientation splits resizes on both axes (tmux semantics); the first of `-L -R -U -D` wins, and a flag without a number means 5 cells (tmux's default). The absolute form `-x COLS` and/or `-y ROWS` sets exact extents and cannot combine with the direction flags. Both forms re-fit the affected panes' terminals and PTYs to the layout geometry.
- **`resize-pane -Z`** toggles zoom (tmux semantics): the pane's terminal and PTY take the full window grid (less the window's declared per-pane chrome, see `refresh-client -I`) while the layout tree is untouched, so `-Z` again restores the exact prior geometry. On the wire, `window_layout` keeps the true tree (that is what makes unzoom exact), `window_visible_layout` is the zoomed pane alone at full extent, and `window_raw_flags` carries `Z`. Zooming a different pane moves the zoom to it; `split-window`, `kill-pane`, `swap-pane`, `break-pane`, `join-pane`, and `select-pane` to another pane unzoom first (selecting the zoomed pane itself keeps the zoom). A successful `resize-pane -x/-y/-L/-R/-U/-D` also ends the zoom before it moves a divider, as tmux does, so the hidden layout never changes under a zoom; a rejected resize (no divider on that axis) changes nothing, including the zoom. `-x` and `-y` together are all-or-nothing: if either axis cannot move, neither does. Zoom is session state, not layout — a restored window starts unzoomed. `-Z` cannot combine with the other adjustment forms.
- **`break-pane` and `join-pane` move panes between windows without re-spawning them** — the pane's process, terminal, and title travel with it; only layout membership changes. `break-pane -s %N` puts the pane in a new window appended to its session (the session's new active window, inheriting the source window's grid size); the source window closes when the pane was its last, but the session cannot die — the new window exists before the source drops. `join-pane -s %N -t %M` moves a pane next to the target with `split-window`'s arrangement rule (`-h` beside, `-v`/default below; `-p` is the moved pane's share, default 50) and makes it the destination's active pane; a same-window join is a within-window move. The mirror image of break-pane's guarantee: joining the last pane out of a session's only window **closes that session** (the destination belongs to the target's window). Both end any zoom in the windows they touch.
- **`move-window -s @N -t <position>` and `swap-window -s @A -t @B`** reorder the session's window list (the order `list-windows` returns and persistence saves). `move-window` clamps out-of-range positions to the ends; `swap-window` requires both windows in the same session. The active window is tracked by identity, not position — it stays active through the reorder.
- **`capture-pane -S/-E`** use tmux's offset convention: `0` is the first visible line, negative numbers count history lines back from the screen top. Without flags, the visible screen is returned.
- **`wait-output` replaces polling `capture-pane`** (herdr `pane wait-output` parity). It matches against exactly what `capture-pane -S/-E` returns for the same range (one shared content function), one line at a time with trailing blanks trimmed: `-m TEXT` is a literal substring, `-r REGEX` a Rust `regex` pattern (an invalid one is a parse error). Without `-S` it searches the last 100 history lines plus the screen, so a marker printed and scrolled off within one burst still matches; `-S 0` narrows to the screen. Output already present matches at once, so wait for a unique marker rather than a prompt that may already be on screen. The reply body is the first matching line. Failures are `%error` blocks with stable prefixes: `wait-output: timed out after <ms>ms without a match on %N` (`-T` elapsed; `-T 0` is a one-shot check), `wait-output: pane %N exited without a match`, `wait-output: pane %N closed without a match`, `wait-output: the daemon is shutting down`, and `wait-output: client disconnected` (the issuing client hung up mid-wait, noticed once it leaves the broadcast registry). Without `-T` it waits until one of those. Through `--cmd` a match exits 0 and prints the line, any `%error` exits 1, and a transport failure exits 2; `--cmd` stretches its reply deadline to `-T` plus 5 s (unbounded without `-T`), and drops the `%output` the daemon pushes meanwhile, so a long wait holds bounded CLI memory. The wait parks only its own connection: it holds no lock while parked (it sleeps on the pane's update signal and re-checks liveness every 200 ms), so other clients are served normally, but a later command on the same connection queues behind it.
- **The agent lifecycle verbs** (parity with herdr's `agent start`, waiting `agent prompt`, and `agent explain`) drive an agent through the same detection tiers the roster reads, with no new state: a hook claim or a scrape rule is the only evidence.
  - **`agent-start -t %N [-T ms] [-x program] <kind> [arg …]`** launches an agent into a pane that sits at its shell prompt. It refuses a pane already carrying an agent label, and a pane whose foreground process is not a shell (re-checked for 2 s while a shell finishes starting; Windows cannot read the foreground, so the check is skipped there). It tags the pane's `agent` label with `<kind>` at once, so the scrape tier watches it, types `<program or kind> <args>` (each word shell-quoted) plus Enter, and replies `%N <agent> <state> <source>` only once a hook report or scrape rule stores a ready state (`idle` or `done`) under that label (or an alias of its pattern set, `claude-code` for `claude`). A `working` state is startup still running and keeps the wait going, as in herdr, so the next `agent-prompt -w` cannot mistake the end of startup for its own turn. `-x` types a different program while keeping `<kind>` as the label. `-T` is the detection deadline, 1 to 300000 ms (default 30000). Failures are `%error` blocks prefixed `agent-start:`: `timed out after <ms>ms waiting for <kind> on %N` (with ` (still working)` appended when the agent was detected but never settled), `… is blocked (or asking a question) during startup and is not ready for prompts`, `pane %N exited before <kind> was detected`, `expected <kind>, detected <other> on %N`, and the busy-pane refusals. A start that ends without detection removes the tag it wrote, unless a hook claimed the pane meanwhile. droid, copilot, and cursor have no state hook (their session hooks report identity only) and draw no idle marker, so their start is confirmed by the scrape tier's idle fallback (rule `idle_fallback`: no rule matches and the agent's process runs in the pane). A fallback `idle` is accepted only 3 s after the command was typed (herdr's start settle), since a TUI still drawing its first frame matches no rule either; a deadline inside that window fails with ` (still settling)` appended, so `-T` must exceed 3000 for these three. What this confirms is that the agent's process is running and shows no working or blocked screen, not that its prompt is drawn. On Windows (no process table) the fallback never fires, so their start still times out there, as does any agent with no state hook, no `idle` rule, and no fallback, or without a pattern set. On Windows an argument containing `"` is refused, since no quoting carries it through both cmd and PowerShell. The label persists, so `agent-start` saves state.
  - **`agent-prompt -t %N [-w] [-u state,…] [-T ms] [--] <text>`** submits `<text>` (the raw rest of the line, so no quoting is needed; a lone quoted word is unquoted, and `--` lets the text start with `-`) to a pane with an agent label. The text goes as a bracketed paste when the agent enabled bracketed paste, then Enter 300 ms later, so a TUI that reads a fast paste as typing commits it first. A `blocked` or `question` agent is refused (a question needs the same human answer). Without `-w` the reply is empty once the text is written. With `-w` the command blocks until the agent's stored state is one of `-u` (default `idle,done,blocked,question`), replying `%N <agent> <state> <source>`. **Anti-stall gate:** the wait counts only after evidence the prompt landed, meaning an entry into `working`, `blocked`, or `question` after submission (a counter, so a working-to-idle bounce faster than the 100 ms poll still counts, and the daemon's `done`-to-`idle` seen flip never does), or an agent that was already `working`. With no such evidence within 5 s the reply is `agent-prompt: stalled: no working or blocked state on %N within 5000ms of the prompt; current state is <state>`, instead of mistaking the unchanged idle state for a finished turn. `-T` bounds the whole wait (`agent-prompt: timed out after <ms>ms on %N; current state is <state>`); without it the wait lasts as long as the agent works. The wait also ends on pane exit, pane close, the agent label changing, the daemon shutting down, or the client hanging up.
  - **`agent-explain -t %N`** replies `key value` lines: `pane`, `agent`, `state` (`unknown` when nothing is stored), `source` (`hook`, `scrape`, or `none`), `hook-source` and `hook-seq` (the reporting script's tag and its last sequence number) when a hook reported, `rule` (the scrape rule that set the stored state), `patterns` (`bundled:patterns/<id>.toml` or the override file's path, or `none` for an agent without a pattern set) and `patterns-version`. For a scraped or unclaimed pane it then evaluates the live screen and title: `rules-evaluated <n>` and `live-rule <id> state=<s> priority=<p> region=<r>` (or `live-rule none`). A hook-authoritative pane is never scraped, which the reply says as `scrape skipped hook-authoritative`.
  - **`adopt-pane -t %N <kind>`** makes a pane the user launched an agent in by hand visible to the agent layer: it writes the `agent` tag `agent-start` writes, minus the launch — the pane keeps its process and the scrape tier gains it, exactly like a TUI-started pane. The kind must have a scrape pattern set (the tier is what keeps the row populated; anything else is refused). It classifies the pane once, immediately — the reply is `%N <kind> <state> scrape` from that read (e.g. `%5 claude idle scrape`), or `%N <kind> unknown none` when no rule matches yet (the tick keeps trying; `unknown` never becomes a guessed `idle`, and the roster shows no row until there is a state). It refuses a pane that already hosts an agent. Adoption never becomes hook authority — the pane's own session hook still claims it when its agent reports. **The visibility contract this closes:** a pane reaches the roster through exactly four doors — the TUI's agent-start, a session hook report, `adopt-pane`, or the restore re-deriving a scrape-tier label from the pane's respawn command (an adopted pane's label is not itself persisted — the save carries identity only — so restore classifies the spawn command's first token, basename, under the same scrape-pattern gate `adopt-pane` applies, and the 1s tick rewrites the state; a nested `sh -c` launch still needs the adopt, and a dead pane is never re-labeled; pinned by `a_restart_relabels_an_adopted_pane_from_its_invocation`); a claude started by hand in a plain shell pane is invisible until one of those fires, and claude reads hooks at startup, so sessions that predate the hook install need the adopt (or a `claude --resume` restart, which fires SessionStart again). The claude hook script also registers the mid-flight events (`prompt` on UserPromptSubmit → working, `notify` on Notification → question when the message names a permission prompt else idle, `stop` on Stop → idle), so a live session re-registers between its start and end even when SessionStart was missed.
  - Both blocking verbs park only their own connection, holding no lock while waiting (they re-read the pane's claim every 100 ms under a brief tree lock), and `--cmd` stretches its reply deadline to cover them: `agent-start` gets `-T` (or 30 s) plus 5 s, and `agent-prompt -w` gets `-T` plus 5 s, or no deadline without `-T`. Through `--cmd` a success exits 0 and any `%error` exits 1.
- **`capture-pane -e`** returns the pane's visible screen (the active grid — an alt-screen TUI captures its TUI screen) as one line per grid row with SGR escape bytes inline, tmux's `-e` contract: a styled run is emitted as `\x1b[0;<fg>;<bg>[;<attrs>]m` before its text (the export walker's fixed reset-fg-bg order), a row that used any SGR ends with `\x1b[0m` before its newline, and a row that used none is plain text. Empty rows are empty lines, so the reply always holds exactly one line per grid row. Styled trailing blanks survive (plain text trims them). Without `-e` the reply stays the plain logical-lines capture, byte-identical to the pre-`-e` reply; `-e` composes with `-S/-E`, which then trim the styled scrollback+screen composition. The framing is line-per-row, not cursor addressing — replaying it into an emulator requires per-row addressing on the consumer side (a bare LF staircases).
- **`send-keys`** speaks the tmux contract: key names, `-l` literal payloads, `-H` hex bytes, and bare `0xNN` tokens. No trailing newline is appended — `Enter` is an expressible key. **Whitespace-separated payload tokens join with NOTHING between them in every mode, exactly as real tmux (3.7c, measured)**: `send-keys -t %0 -l echo LEFT` types `echoLEFT` (the separator between `-l` tokens is dropped — a common surprise), while the quoted single argument `send-keys -t %0 -l 'echo LEFT'` types `echo LEFT`. A space must live inside a quoted run or be an explicit `Space` key name; for bytes a shell line cannot express (including arbitrary spacing and control bytes), `send-keys -H 20 0a …` is the lossless path. Key names come in two classes. The control-byte names (`C-a`…`C-z`, `C-Space`, `Escape`/`Esc`, `BSpace`, `Space`, `Enter`, `Tab`) are always the raw byte, because par-term sends already-encoded keystrokes in this form. The navigation and function keys (`Up`/`Down`/`Left`/`Right`, `Home`, `End`, `PageUp`/`PgUp`/`PPage`, `PageDown`/`PgDn`/`NPage`, `Insert`/`IC`, `Delete`/`DC`, `F1`…`F12`, `BTab`) go through the shared key encoder against the target pane's application-cursor (DECCKM) and kitty keyboard state, as tmux does: `Up` is `ESC [ A`, or `ESC O A` once the app enables application cursor keys. A key name matches inside a quoted run too (`'Home'` is the Home key, as in tmux), so use `-l` to send such a word as text. Any other token is written literally.
- **`refresh-client -C WxH`** reports the client's renderer size — the connection's sizing contribution. A window's extent is the **componentwise minimum over the clients currently displaying it** (each viewer's reported `-C` size; the per-window form of tmux's `window-size smallest`): every viewer sees the whole window, and a client larger than the window pads the space past it with dim `.` cells, as tmux does. A smaller client attaching shrinks the window, and a constraining client disconnecting or switching its view away re-fits it to the remaining viewers' minimum. A report that does not move the window (a viewer above the minimum, or a repeat) answers the reporting client alone, with no `%layout-change` broadcast, so two differently-sized clients cannot re-report each other into a re-seed loop. A window nobody displays keeps its extent until next displayed.
  Repeated identical reports are idempotent (the re-fit fires only on an extent change), so the reports the reseed and attach handshakes replay never feed back into a resize loop.
  Only reported sizes constrain — a control or hook connection that never sent `-C` holds no contribution. Never persisted; a reconnecting client re-reports on attach.
  The `-t`-less form is the size report the attach handshake sends before it has resolved a pane: it records the connection's view as the newest session's active window (the same stand-in bare `new-window` uses) — never a replay, since replay is what the `-t` form exists for.
  With `-t` and without `-C`, the command replays the pane's state to the requesting client — the reattach resync. The reply is `Terminal::export_screen_restore_sequence()`: the main screen's scrollback first when the pane has any (`\x1b[H` anchor, each history line CR-led and LF-terminated so it replays into the client emulator's own scrollback in order, then the still-on-screen lines pushed off the bottom row with plain line feeds), alt-screen selection next (`\x1b[?1049h` when active), the scroll region, then the styled screen content with absolute row addressing (`\x1b[H` anchor, per-row `\x1b[R;1H`, SGR-diffed runs, reset per row), then the cursor position (`\x1b[R;CH`), visibility (`\x1b[?25l`) and style (DECSCUSR), the input modes (DECCKM, bracketed paste, focus tracking, mouse tracking/encoding), origin mode restored last with a region-relative re-position, and a final `\x1b[0m`.
  The scrollback block does carry `\n` bytes (one per replayed history line) — despite that, raw ESC bytes still survive the `%begin`/`%end` line framing untouched, since the reader consumes the reply as an ordinary sequence of framed lines regardless of how many lines the body spans; a client feeds the whole body verbatim into its pane emulator and the pane's subsequent `%output` deltas land on the restored state.
  A pane with no scrollback yields the pre-scrollback-fix shape: no `\n` at all, purely CUP-addressed.
- **`refresh-client -p WxH`** reports the client's per-CELL pixel size (font metrics, e.g. `-p 10x20`) — the one renderer measurement every pane shares, while grid extents differ per pane. It is held daemon-wide with the same latest-wins policy as `-C`, applied to every pane through `resize_with_cell_pixels`, and re-derived on every later re-fit, so `TIOCGWINSZ` (image tools read it), XTWINOPS pixel reports (`CSI 14 t`/`CSI 16 t`), and image cell-span math (how many rows the cursor advances after a kitty/iTerm2/Sixel image) all carry the size cells actually render at instead of the 10×20 construction default. Independently optional from `-C`: pixels without `-C` re-fit at the current grid, `-C` without pixels keeps the last reported cells. **Client contract:** send `-p` with the cell size on attach and whenever the font or zoom changes, alongside the `-C` report a resize already sends. Not persisted — a reconnecting client re-reports.
- **`refresh-client -C WxH -I border=B,gap=N,gutter=G`** declares the per-pane chrome the client paints inside every pane rect (feature token `chrome`, card 01a11c5b): `border=1` is a one-cell ring on every side, `gap=N` an N-cell band per side, `gutter=1` one right-edge column.
  Every key is optional (absent = none). An unknown key or a value other than `0`/`1` (or a number, for `gap`) rejects the report.
  The daemon reserves the chrome the way tmux reserves border lines: pane rects, and so the layout string, keep their full geometry, and each pane's PTY is the rect less the chrome.
  The gap clamps so one cell of edge survives; the ring applies only when the gap-inset rect is larger than 2x2 (a smaller pane keeps its full area); the gutter takes one more column; the PTY floors at 1x1.
  So a bordered single pane in an 80x22 content grid runs at 78x20, and `pane-info`/`list-panes` report that interior. The declaration is part of the connection's sizing contribution, beside its `-C` size: a window reserves the union of its viewers' declarations (ring if any viewer has one, the largest gap, gutter if any), so the PTY is never larger than a declaring client's painted interior, and a chrome-only change (a border toggle at an unchanged host size) re-fits like an extent change.
  A report without `-I` declares no chrome, which is exactly the pre-`chrome` sizing. **Client contract:** send `-I` on every `-C` report when the daemon advertises `refresh-client chrome` and the client paints any chrome (re-send on a runtime chrome change); size the pane emulators to the same interior.
  The `par-mux attach` render client computes its paint/mouse/cursor inset and the declaration from one function (`PaneChrome::interior`), so the painted interior and the PTY grid are always equal.
  **Known limitation:** with two viewers that paint different chrome, the one with less chrome paints a view larger than the PTY (the union is reserved), so its pane shows a blank margin instead of a crop.
- **`set-client-colors -f/-b rrggbb`** reports the client's theme foreground/background so OSC 10/11 queries in panes answer with what the client actually renders (dark/light detection) instead of the core's built-in theme. Each flag is independent; at least one is required; six hex digits, no `#`. Applied to every pane terminal and inherited by panes created later. **Client contract:** send it on attach and on theme change. Not persisted.
- **Kitty `t=t` (temp-file) images** render in mux panes because the daemon-side pane terminal *retains* the temp file: client mirrors rebuild their grid from the raw `%output` bytes, and the t=t contract has the reading terminal delete the file — the daemon reads first, so its read must not delete, or no client could ever load the graphic. The client that actually renders the graphic deletes it with its own read. Streamed (`m=1`) and direct transfers carry their data inline and never had the problem; `t=f` files are never deleted by anyone. Two attached clients remain first-reader-wins for t=t (the second mirror finds the file gone) — same as before, no worse. A t=t file whose client never processes it lingers in the sender's temp dir until OS cleanup, the one accepted leak.
- **Session environment (`set-environment`, `new-session -e`)** is a per-session variable map applied on top of the daemon's own environment for every pane spawned into that session afterwards. Panes already running are untouched (tmux semantics), and the `PAR_MUX_*` contract vars always win over a same-named session var. It exists because the daemon outlives its clients: panes otherwise inherit whatever environment the daemon was started with, so a client that reconnects over SSH or with a changed `SSH_AUTH_SOCK`, `DISPLAY`, or `PATH` sends its current values here (tmux's `update-environment`). `-t` is required, `-e` repeats, quoting follows the bounded grammar above (`-e 'A=two words'`, `NAME 'a value'`), and a name that is empty or contains `=` or NUL is refused. Values can be secrets: they are never logged, and they persist in the owner-only state file.
- **Buffers** are a single slot named `default` — no numbered stack, and `-b` is not implemented.
- **`kill-pane`** of a window's last pane closes the window (`%window-close`), and of a session's last window closes the session (`%sessions-changed`). A pane whose child process exits on its own (rather than via `kill-pane`) is HELD with its exit code (remain-on-exit) and announced, or REMOVED through this same contract when `[daemon] remain-on-exit` is off (the default) — the removal runs the full cascade, so a workspace's last pane closes its workspace too (Paul's rule, 2026-10-18: a pane closes when its child exits; the keep-one-tab hold survives as the `set-workspace-respawn` opt-in) — see [Pane Reaping](#pane-reaping) below; under the hold, `respawn-pane` restarts it in place.
- **`kill-server`** raises the same shutdown flag SIGTERM does: the reply goes out, the accept loop notices on its next tick, `%exit` reaches every client, and the final state save runs before the process exits — see [Shutdown Semantics](#shutdown-semantics). Refused with an error for an embedded `MuxServer` that has no shutdown handle (`ctx.shutdown` is `None`). `par-mux --stop`/`--restart` send this command under the hood.
- **`list-agents`** returns one line per pane a hook has claimed or a scrape pattern has matched, sorted by pane id: `%N <agent> <state> <source>` with `source` `hook` or `scrape`, then zero or more whitespace-free `key=value` tokens: `reason=<base64>` (the blocked reason — standard base64 of the whitespace-collapsed message, so a reason reading `hook` or containing `telemetry=` cannot be mistaken for the grammar's own tokens; ARC-060), one `telemetry=<base64>` token when the pane holds a **fresh** telemetry sample (base64 of the canonical JSON, so string values with spaces ride as one whitespace-free token), and a sibling `host_telemetry=<base64>` token when the daemon's host probe has fresh fields for the pane's cwd, and a `ws=<id>` token naming the pane's owning workspace (`+N`, the attach side panel's agent-grouping key). **Consumers parse positions 1-4 positionally, then split each remaining token on its first `=`; unknown keys are ignored.** Stale or absent telemetry adds nothing — the row keeps its exact pre-telemetry shape — and a token leaves with its sample's age, no write needed. Panes without state are absent — `unknown` is never reported, and `idle` is never guessed (a set's process-gated `idle_fallback`, see Agent Scrape Tier, is the one exception). The state is one of `working`, `blocked`, `question`, `done`, `idle` (see the next bullet for `done`); `question` marks a dialog asking the user to choose (permission prompts, pickers) and is blocked-class for the prompt verbs, sounds, and notifications.
- **`done` is finished-but-unseen** (herdr's `done`), decided daemon-side so every client agrees: an `idle` (hook report or scrape match) that follows `working`, `blocked`, `question`, or `done` while no render client views the pane is stored as `done`. A pane is viewed when it is the pane some viewer shows: the zoomed pane, else the active pane, of the viewer's focused window. A viewer is a connection that has sent a size report (`refresh-client -C`); one-shot CLI and hook connections never view anything. A viewer's focused window is the one it last selected for itself (`select-window`, `switch-client -t`, `select-workspace`), a shared-selection follow, or its latest targeted size report, whichever came last, except that a report naming a different window never moves a focus the viewer's own selection placed: that report was sent for the window it just left. Only a dead focused window yields to such a report. The target-less handshake report sets the focus only when the viewer has none, or has only an earlier target-less one. Under the pane rule a passthrough attach, which reports its whole window while showing one pane (and selects that pane before every report), views that pane alone, and a render client viewing a split window sees only its focused pane's finish; the unfocused splits keep `done` until focused (herdr marks its whole active tab instead). After every control command the daemon clears each viewed pane's `done` to `idle` and broadcasts the flip as `%agent-state-changed` with the claim's own `source=`; a finish that lands while the pane is viewed is `idle` directly. A hook or scrape rule may also assert `done` outright (stored as `idle` when the pane is already viewed). A consumer waiting for an agent to finish must accept `idle` or `done`: with no render client attached every finish reads `done`. Limits: the host terminal's focus (mode 1004) is not consulted, so a finish in the focused pane of an attach whose terminal is in the background still reads `idle`. A viewer that moves by `select-pane` alone (no window selection) changes its focused pane, not its focused window. Pinned by `done_holds_until_a_client_views_the_pane_then_clears_to_idle`, `a_late_size_report_for_a_left_window_does_not_mark_it_seen`, `a_late_size_report_after_a_session_switch_does_not_mark_it_seen`, `a_single_pane_view_does_not_mark_its_siblings_seen`, `a_zoomed_window_views_only_the_zoomed_pane`, `a_report_after_the_focused_window_dies_moves_the_focus`, `a_finish_out_of_view_is_done_until_a_client_views_the_pane` and `a_scraped_finish_out_of_view_is_done_and_holds_quietly`.
- **`list-commands` is capability discovery** (ENH-037). The reply is one line per dispatchable command, sorted: `name [feature …]`, where feature tokens name flag-level abilities a client cannot probe by sending — `resize-pane zoom absolute`, `split-window before start-dir`, `respawn-pane kill start-dir`, `pane-info cmd`, `refresh-client cell-pixels chrome`, `capture-pane escape` — plus one daemon-level trailing line, `features replay-held-state`, announcing registration-time replay. Tokens are `[a-z0-9-]+` (numerals allowed since ENH-043's `client-snapshot v1`); a client must ignore unknown tokens and lines, and a daemon never removes a token without a CHANGELOG "Removed" entry. **Client contract:** send `list-commands` once after `version`; an `unknown command: list-commands` error means a pre-feature daemon — assume the older feature set known from its `version` stamp, or none. The tokens are the third column of the `COMMANDS` table itself (`crates/par-mux/src/mux/command/mod.rs`), one declarative row per command (ARC-094b).
- **The command schema is the machine-readable form of that table** (herdr `api schema` parity). `list-commands -J` replies with one JSON document (the reply body spans several lines; join them), and `par-mux --print-schema` prints the same bytes from the binary with no daemon running; Rust embedders read the `par_mux::mux::command::COMMAND_SCHEMA_JSON` constant. The committed file is [`crates/par-mux/schema/par-mux-commands.json`](../crates/par-mux/schema/par-mux-commands.json). Shape: `{"version": 1, "format": "par-mux-command-schema", "source": …, "daemon_features": [...], "commands": [{"name", "features", "flags", "synopsis"}, …]}`, commands sorted by name. `name` and `features` come from the `COMMANDS` table; `flags` (every single-dash and double-dash flag the command accepts, sorted) and `synopsis` (the Arguments cell, backticks stripped) come from this Command Reference table, which `make mux-docs-check` already ties 1:1 to `COMMANDS`. Versioning: added keys keep `version` 1, a removed or renamed key bumps it, and a consumer must ignore keys it does not know. The file is generated, never hand-edited: `make mux-schema` rewrites it, and `make mux-schema-check` (part of `checkall`) fails when it drifts from the table or this reference — so a new command or feature token ships with its schema entry. Prefer `-J` against a live daemon (it describes what that daemon parses) and `--print-schema` for what the local binary would parse. **Client contract:** send `-J` only after bare `list-commands` advertised `list-commands schema` — a daemon older than the schema ignores the flag and answers with the token lines instead of an error.
- **`pane-info` viewer status** adds `constraint_cols=<kind:id|->`, `constraint_rows=<kind:id|->`, and `viewers=<comma-separated attach:id entries>` tail tokens. Constraints are window-scoped componentwise minima, ties broken by lowest connection id, while the fixed prefix still gives the effective pane interior. A client can constrain one axis without constraining the other. With no reports, owners are `-` and the viewer list is empty. `-M` marks the issuing connection as a streaming mirror until disconnect: it may still constrain size (`stream:id`) but is not counted as an attach viewer. The streamer combines the daemon attach list with its own websocket presence. Querying again observes changes even when the grid is unchanged. Unknown tail tokens must be ignored.
- **`client-snapshot -t <window>` is the attach client's one-round-trip status read** (ENH-043). The reply is one row per line, the first word naming the row's kind, in this order:
  ```text
  snapshot 1
  workspace +W <*|-> <name>
  session $N <*|-> +W <name>
  window @N <*|-> <name>
  pane %N [<title>]
  agent %N <agent> <state> <source> [key=value…]
  worktree +W <repo|worktree> <+P|-> <branch|-> <checkout>
  ```
  `workspace` rows list every workspace in id order, `*` marking the daemon's active one. `session` rows list every session in `list-sessions` order (workspace order, then session order), `*` marking the session that owns `<window>`, `+W` its workspace. `window` rows are the owning session's windows in window order, `*` marking its active window. `pane` rows are the panes of `<window>` in layout order, each title the pane's effective title (`pane-title`'s value) with every `\r` and `\n` replaced by a space so one title stays one row; an empty title is the bare `pane %N`. `agent` rows are exactly the `list-agents` rows, `agent `-prefixed, sorted by pane id. `worktree` rows are exactly the `list-worktrees` rows, `worktree `-prefixed, id order (the side panel's worktree grouping); none when no workspace is worktree-backed. In every row but `agent` and `worktree` the name or title is the rest of the line, so spaces survive. A window target that does not resolve gets the resolver's `no such window …` error; a window no session owns gets `no session owns <window>`. **Version rule:** `snapshot 1` is the format version. A client ignores unknown row kinds and unknown trailing `key=value` tokens; a daemon only appends new row kinds and never changes an existing row's positional fields without bumping the version. **Client contract:** send `client-snapshot` only when `list-commands` advertises the `v1` feature token on its row (`client-snapshot v1`); otherwise fall back to the individual `list-workspaces`/`list-sessions`/`list-windows`/`pane-title`/`list-agents` queries. It is a query: it never triggers a state save.
- **Pane titles (`select-pane -T`)** set a user title on the pane; `-T ''` clears it (quoting is what makes an empty value expressible). Precedence is a deliberate divergence from tmux: a user title is **sticky** — the pane program's OSC 0/2 title never overwrites it — while with no user title the pane reports the program's live OSC title. The effective title (user when set, else OSC) is what `pane-title -t %N` replies — an empty reply body means neither is set; a broadcast carries only the *user* title's changes, so a client composing a display title falls back to its own OSC tracking on the empty form. Setting the same title again is a no-op and broadcasts nothing.

## Git worktree workspaces

`worktree-create`, `worktree-open`, `worktree-remove`, and `list-worktrees` (see the [Command Reference](#command-reference)) make a git worktree a workspace, the way herdr's `worktree create/open/remove` does:

```bash
par-mux --cmd "worktree-create -c ~/Repos/app -b feat/login"   # +3 (repo workspace "app" is +2)
par-mux --cmd "worktree-open -c ~/Repos/app-worktrees/feat-login"
par-mux --cmd list-worktrees
# +2 repo - main /Users/me/Repos/app
# +3 worktree +2 feat/login /Users/me/Repos/app-worktrees/feat-login
par-mux --cmd "worktree-remove -t +3"        # refuses a dirty checkout or an agent pane without -f
par-mux --cmd "kill-workspace -t +2 -g"      # -g closes app and every linked worktree workspace
```

- **Branch flag.** The branch is `-b <branch>`, as in `git worktree add -b`. `-n` names a workspace on `new-workspace`, so `worktree-create` refuses `-n` and points at `-b` ([D7](MUX_DECISIONS.md)). The start point is `--base <base>` (long-only): git's `worktree add -B` means create-or-reset, so a short `-B` here would be read as that; it is refused with a pointer to `--base`.
- **Checkout layout.** Without `-p`, a new checkout goes to `<parent>/<repo>-worktrees/<branch-slug>`, a sibling of the repo's MAIN checkout (the slug lowercases the branch and collapses every non-alphanumeric run to `-`, herdr's `branch_to_path_slug`). herdr's default is one global root (`~/.herdr/worktrees/<repo>/<slug>`); par-mux keeps checkouts beside the repo so they share its filesystem and never appear in the main checkout's own `git status`. Paths given to `-c`/`-p` must be absolute or `~/`-relative — the daemon's cwd is not the client's.
- **Grouping is workspace metadata, not a naming convention.** A worktree workspace carries a link: the repo's canonical main-checkout path (the group key), its own checkout, and its branch. The repo workspace is the one whose checkout IS the main checkout; linked worktree workspaces share its key. The group is derived from that key, never stored as a parent id, so a linked workspace whose repo workspace was closed regroups under the next one `worktree-create`/`worktree-open` opens for that repo. Names are display-only (default `<repo>` and `<repo>/<branch>`, with a `-2`, `-3`, … suffix when taken) and renaming a workspace never ungroups it.
- **Closing.** `kill-workspace` of a repo workspace with open linked worktree workspaces is refused without `-g` (herdr's group-close rule for `workspace close`); `-g` closes the group. A linked worktree workspace closes alone without a flag. Closing never deletes a checkout — `worktree-remove` does, and closes only that workspace. Only `kill-workspace` is gated: a cascade that empties the repo workspace — `kill-session`, `kill-window`, or `kill-pane` of its last session, window, or pane, a last pane exiting, `kill-server`, an exit-when-empty shutdown — closes it without `-g`, and its linked workspaces stay open ungrouped until the next `worktree-create`/`worktree-open` for the repo reopens a repo workspace (`list-worktrees` shows their parent as `-` meanwhile).
- **Removal guards.** `worktree-remove` refuses, unless `-f`, when the checkout's `git status` (untracked files included) reports anything or a pane in the workspace carries an agent claim — any pane `list-agents` lists, whatever its state: an `idle` or `done` agent is still a live process in the checkout. git runs before the workspace closes, so a refusal or a git failure changes nothing. On Windows a live shell whose cwd is the checkout blocks deletion, so a first refusal there closes the workspace and retries for about three seconds (the workspace stays closed if git still refuses).
- **git hygiene.** git runs through argv only (no shell), with `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` cleared, `GIT_TERMINAL_PROMPT=0`, null stdin, and `LC_ALL=C`; a branch or base starting with `-` and a branch name `git check-ref-format` rejects are refused before anything runs. The repo's hooks stay live (an explicit user request, unlike the [host probe](#host-telemetry-probe)'s unrequested reads). git's multi-line diagnostics are collapsed to one `%error` line. `--cmd` gives `worktree-create`/`worktree-remove` a 10-minute reply budget, since a large checkout outlasts the default.
- **Persistence.** The link persists with its workspace (an optional `worktree` object in the state file; format version unchanged), so groups survive a daemon restart. A checkout deleted while the daemon was down still restores its workspace (the pane lands in `$HOME` with the usual gone-cwd note); `worktree-remove` then prunes the stale entry.

## Raw pane streams

A local client can follow one pane's raw output, and optionally drive it, without parsing the whole broadcast stream (herdr's `terminal session observe` / `terminal session control`). Two control commands turn the connection that sends them into a pane stream, and two subcommands wrap them for the shell:

```bash
par-mux observe -t %3              # the pane's screen, then its live bytes, to stdout; read-only
par-mux observe -t %3 --no-seed    # live bytes only
par-mux control -t %3              # the same output, plus stdin forwarded to the pane until EOF
par-mux control -t %3 --size 120x40   # report a fixed grid instead of following this terminal
```

`observe` and `control` take the client targeting trio (`[NAME]`, `--socket`, `--state-dir`). They exit 0 when the daemon ends the stream with `%exit` (the pane closed, or the daemon shut down) or `control`'s stdin reached EOF; 1 when the stream is refused (no daemon, no such pane, or a hook-only pane socket under `--pane-endpoints`, which closes without a reply — pass `--socket`); and 2 when the connection drops mid-stream without `%exit` (the observer was evicted for not draining, or the daemon died) or fails. `control` forwards stdin as a byte stream, as it arrives, in chunks of at most 4 KiB. It does not put the terminal into raw mode, so it is meant for pipes and scripts (`printf 'make test\r' | par-mux control -t %3 --no-seed`). Typed at a terminal, the tty echoes the input into the same stdout and delivers it a line at a time, and Ctrl-C ends the controller. For interactive single-pane use, run `par-mux attach -t %N`. `control` follows the terminal's size by re-reading it every 250 ms (stdout, then stdin; Unix) and reports each change. On Windows, or with no terminal, only `--size` reports a size.

**The wire.** `observe-pane -t <pane>` (or `control-pane -t <pane>`) is answered with one reply block whose body is a header line `pane %N COLSxROWS` (plus ` exited=<code|?>` for a pane already held dead), then the pane's screen-restore seed — the same bytes `refresh-client -t` returns, so feeding the seed to a fresh emulator of that size reproduces the screen, scrollback, cursor, and modes. Strip exactly one header line and join the remaining body lines with `\n` to recover the seed. From then on the connection receives only:

- `%output %N <data>` for that pane — raw PTY bytes, octal-escaped exactly as on the shared broadcast;
- `%pane-exited %N [code]` and `%pane-respawned %N` for that pane;
- `%exit` — the pane left the tree (killed, or removed when its process exited without remain-on-exit) or the daemon is shutting down. The daemon closes the connection after it;
- the reply blocks of the connection's own later commands.

Layout, session, agent, and other panes' lines never reach a stream connection. Frames are raw PTY bytes rather than rendered lines or screen diffs: the consumer runs its own terminal emulator (the seed initializes it), which keeps the stream lossless for escape sequences, graphics, and modes, and costs the daemon nothing beyond the fan-out it already does.

**Ordering rule for consumers.** Ignore every line that arrives before the stream reply's `%end` — pre-reply `%output` (any pane), the registration replay of held panes and zoomed windows, other broadcasts — then apply the seed, then apply every later `%output` in order. The daemon reads the seed and queues it, together with the switch to the narrowed stream, while holding the lock every output push takes, so a pre-reply `%output` is always already contained in the seed and no post-reply one is. The one residual window is the one the attach client's seed shares: the PTY reader applies a chunk to the pane's terminal before forwarding it, so a chunk applied just before the seed is read but forwarded just after can appear twice. `par_mux::mux::pane_stream` (`StreamDecoder`, `StreamReader`, `open`, `run`) implements the rule for Rust embedders; `MuxClient` does not, because it splits replies and notifications onto separate channels and loses their relative order.

**Multiple observers.** Each stream is an ordinary registered connection whose writer filters the shared broadcast, so any number of observers and controllers can follow the same pane at once. Each has its own bounded queue and the usual [eviction](#client-backpressure-and-eviction) rule, so a stalled observer is disconnected without slowing the others.

**Read-only vs writable.** On an `observe-pane` connection every later command is refused with `%error observe-pane: this connection is a read-only pane stream`. A `control-pane` connection keeps the full command grammar on the same socket — a sidecar connection or mode switch is unnecessary because the control socket is already full-duplex: input goes through `send-keys -t %N -H <hex bytes…>` (the byte-exact form; `par-mux control` sends stdin this way) and size through `refresh-client -t %N -C WxH`. That size report is the connection's own contribution to the window's smallest-client size, exactly like an attach client's, and it is withdrawn when the connection closes, so the window re-fits to the clients still viewing it. It sizes the pane's WINDOW, not the pane: in a split window the pane gets its layout share of `WxH`. A controller that has reported a size is a full viewer, like an attach client. It is announced to the other clients (`%client-attached`/`%client-left`), and the daemon's seen pass clears a `done` agent to `idle` in that window's active (or zoomed) pane, the one an attach would show, which need not be the streamed pane. An observer, and a controller that never reports a size, does neither. A second `observe-pane`/`control-pane` on a connection that already streams is refused. A pane's death keeps the stream open (the `%pane-exited` line arrives, and a `respawn-pane` continues it with `%pane-respawned` and fresh output); only the pane leaving the tree ends it.

**Limits.** A stream follows one pane; follow several with several connections. There is no takeover: input from a controller interleaves with every other writer to the same pane (attach clients, `send-keys` from scripts), as tmux's does. A controller's input rides the same `send-keys` dispatch every client uses, which writes to the PTY with the tree lock released (the QA-225 snapshot-then-write shape shared with `paste-buffer`) — a program that stops reading its input while a controller keeps pasting stalls only that controller's own connection, not other clients' commands. The stream's SEED export also runs off the clients lock (an export of a deep-scrollback pane would otherwise pause every pane's `%output` fan-out for its duration): the seed is enqueued under the lock, and an output epoch re-checked there forces a re-export when output raced the export, so the seed can never land ahead of output it does not reflect (pinned by `seed_export_racing_output_stays_ordered`). The integration tests run on Unix only. On Windows the stream logic is the same, `par-mux control` reports only `--size`, and after `%exit` the pipe stays open until the client hangs up (`par-mux observe`/`control` do so on `%exit`).

## Notifications

Broadcast lines every connected (registered) client receives, emitted in this order per mutating dispatch: `%layout-change` first (it carries the geometry the pane-changed notification is read against), then lifecycle broadcasts, then issuer-only notifications.

| Line | Meaning |
|------|---------|
| `%output %N <data>` | Pane output, octal-escaped, pushed as bytes arrive — no polling |
| `%layout-change @N <layout> <visible-layout> <flags>` | A window's geometry or zoom state changed. `flags` carries `Z` while the window is zoomed — the per-window zoom truth every client reads (tmux semantics): `resize-pane -Z` broadcasts it in both directions, and the unzooms (`select-pane` away, any layout mutation) broadcast the unflagged line |
| `%window-add @N [layout visible-layout flags]` | A window was created; the layout triple mirrors `%layout-change` and is empty on a bare-id line (real tmux's shape, or an older daemon) |
| `%window-close @N` | A window was killed |
| `%window-renamed @N <name>` | A window was renamed |
| `%window-pane-changed @N %N` | A window's active pane changed |
| `%pane-exited %N [code]` | A pane's process exited: the pane is HELD (remain-on-exit on, or a respawn-on workspace's last pane) or REMOVED through the kill-pane contract (remain-on-exit off, the default — including a workspace's last pane, which closes its workspace with it; the removal's `%window-close`/`%layout-change` queue behind this line). The exit code token is present whenever the OS reported an exit status (a death by signal reports `1`, portable-pty's mapping) and absent only when none was available as the pass observed the death (the PTY closed before the child was reapable). The cue clients show "Process exited (code N)" over — never instead of — the frozen screen |
| `%pane-respawned %N` | A pane's process was restarted in place by `respawn-pane`: same id, window, and layout, fresh terminal — the cue clients clear their exited-state chrome and re-render from the new screen on |
| `%session-changed $N <name>` | Sent to the issuing client after `new-session` |
| `%session-renamed $N <name>` | A session was renamed (`rename-session`). The name is the rest of the line, spaces included |
| `%session-window-changed $N @N` | The shared selection moved: session `$N`'s active window is now `@N` (`select-window`, or `switch-client -t @N`, on the displayed session). Render clients displaying a window of that session follow — the tab-switch sync. A background session's pointer moves silently (no line) |
| `%client-session-changed <workspace> $N <name>` | The displayed session moved: `select-workspace` (or a cross-session `switch-client -t`) landed the shared display on session `$N` (`name` last, spaces included). The first field carries the workspace the display moved to — par-mux has no per-client names. Render clients of the left workspace's sessions follow to the new displayed window |
| `%client-attached <client>` | A registered control client reported its first size (`refresh-client -C`). A one-shot command client or hook-only connection never reports one, so it is never announced. `client` is the daemon's decimal connection id. Sent to the other registered clients, never to the joining client itself |
| `%client-left <client> [$N @N]` | An announced client's connection ended — a clean disconnect, a socket EOF, or an eviction teardown alike — with the session and window it was displaying. A client that was never announced leaves silently. Queued after the `%layout-change` of any window its departure re-fit. tmux's own `%client-detached` stays parse-only |
| `%notification <b64 title> [body=<b64 body>]` | A `show-notification` raised an ad-hoc notification. Title and body ride the standard base64 of their text (one whitespace-free token each; the body token only when set, a trailing key=value a parser skips if unknown). Render clients deliver it through their own `notify-delivery` channel |
| `%config-changed` | A `reload-config` re-read the config file (any client's reload chord, settings-dialog change, or command menu row, or a control client's `reload-config`). Argument-less. Render clients re-read the file and re-apply their `[client]` settings through the reload chord's own rebind, flashing `config re-applied`; a client whose applied `[client]` section already matches the file (the client whose reload caused the broadcast) does nothing. A client never answers it with `reload-config` |
| `%workspaces-changed` | The workspace roster, names, or the daemon's active-workspace pointer changed — `new-workspace`, `kill-workspace`, `rename-workspace`, `select-workspace`, or a session death that removed its emptied workspace. Argument-less, the same convention as `%sessions-changed`: clients re-query `list-workspaces` |
| `%sessions-changed` | The session set changed — a session was created (`new-session`), destroyed (`kill-session`, or a `kill-window`/`kill-pane`/`join-pane` that emptied it), or its windows were reordered (`move-window`/`swap-window`). Argument-less, tmux's shape: clients re-query `list-sessions` rather than parsing the line. **Client contract:** on receiving it, re-run `list-sessions` (and `list-windows` for the new order after a reorder); if the session a view is showing is gone, move that view (the render client lands on its workspace's landing session, else the daemon's active workspace's) or end it when nothing is left (close the tab / show an empty state) — the daemon does not send anything more specific, and if this emptied the daemon it exits shortly after (see [Shutdown Semantics](#shutdown-semantics)), closing the transport |
| `%agent-state-changed %N <agent> <state> [source=hook\|scrape] [ws=+N] [msg=<base64>]` | An agent's state changed, with provenance. Trailing tokens are `key=value`: `ws=` (the pane's owning workspace, the same value `list-agents`' `ws=` reports) always follows `source=` when present, and `msg=` (the state's message — the blocked reason — the standard base64 of the whitespace-collapsed text) follows `ws=` when the hook reported one; unknown keys must be ignored |
| `%agent-released %N <agent>` | The pane's agent released its claim (`pane.release_agent`) — state, hook authority, session identity, and telemetry cleared; the pane left the roster |
| `%agent-telemetry-changed %N <agent>` | An agent's telemetry changed (`pane.report_agent_telemetry` accepted a fresh sample). Identity only, no values — clients re-query `list-agents` for the fresh blob, the same shape `%sessions-changed` teaches |
| `%pane-title-changed %N [title]` | A pane's user title changed — `select-pane -T` set it (title follows the pane id, spaces included) or cleared it (no title token). The user title is sticky over the program's OSC title; the notification carries the user title only |
| `%exit` | Graceful shutdown — the daemon is ending deliberately, not dying. On a [raw pane stream](#raw-pane-streams) it also means the streamed pane left the tree; the daemon closes that connection after it |

tmux control-mode clients ignore `%` lines they do not recognize, so a client that never learned `%agent-state-changed` (or any future variant) still sees the raw line rather than an error.

**Late clients** (ENH-037): on registration a client receives `%pane-exited` for each held pane and `%layout-change` for each zoomed window, queued ahead of its first command's reply. A held pane's `%pane-exited` may repeat: the registration snapshot and an in-flight reap broadcast can overlap, and the line is idempotent (both say "held with code N"). A client that registered before the death or the zoom receives no replay line. At any other time a client can pull the held panes' lines on demand with the `pane-exited-replay` command (ENH-042), delivered to the issuing client only.

## Agent Hook Reports

A pane's process can report agent state over the control socket by sending one JSON line and reading one JSON reply. herdr's integration scripts port unchanged except the env-var rename (`HERDR_*` → `PAR_MUX_*`).

Every pane process is seeded with the env contract (`crates/par-mux/src/mux/pane.rs`):

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

`par-mux --pane-endpoints` changes what `PAR_MUX_SOCKET` names inside a pane: a per-pane socket that accepts exactly the five hook methods below, **bound to that pane** — a report whose `pane_id` names another pane is refused (`pane_id does not match this endpoint`), and a report that omits `pane_id` is filed for the bound pane. Any other line — a control command (`capture-pane`, `send-keys`, `kill-server`, …) — is answered with `{"error":"hook-only endpoint"}` and the connection closes. A pane's child processes can therefore report their agent state but cannot read other panes, type into them, or stop the server. The socket carries the same owner-only boundary as the control socket; it is not a defense against same-uid code, which can always reach the control socket directly (see SECURITY.md).

The mode is **off by default**, and the default stays off: an in-pane `$PAR_MUX_SOCKET` naming the FULL control socket is a core feature — an agent in one pane must be able to spawn and drive agents in other panes via `par-mux --cmd`, and the par-mux skill depends on that in-pane fallback. **Default-flip gate** — the criteria that would ever have to be met to turn the mode on by default: an explicit decision by the repository owner (declined 2026-09-29), acceptance of the break it causes to the in-pane `--cmd` fallback for daemons started without `--expose-control-socket`, and a migration story for the hook shims and agent extensions that consume `PAR_MUX_SOCKET` today. Absent all three, pass `--pane-endpoints` per daemon to opt in, and add `--expose-control-socket` when agent-driven pane control should keep working through `PAR_MUX_CONTROL_SOCKET`.

Five methods (`crates/par-mux/src/mux/hooks/`):

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
- `pane.report_agent` additionally carries `state` (`working`/`blocked`/`done`/`idle`; `unknown` is accepted but never written; a finish nobody saw is stored as `done`, see the `done` bullet), optional identity (`agent_session_id`/`agent_session_path`), and optional `message` — the blocked reason, stored whitespace-collapsed and cleared when a later report omits it.
- Validation is at the door: `state` outside the fixed set, an `agent` label containing whitespace or control characters, or a `source` containing control characters is error-replied, nothing written, nothing broadcast. These fields are interpolated verbatim into the space-split `%agent-state-changed` and roster lines, and any process in any pane can reach this endpoint — a newline in a field would forge a control-mode line delivered to every attached client.
- `pane.report_agent_session` identifies the session by **id or transcript path** (either alone is enough) and may carry `session_resume_argv` — an array of non-empty strings, malformed values error-replied — stored verbatim as the pane's resume invocation. `session_start_source` records startup vs resume provenance. A session report that moves the pane to a **different agent** clears the previous agent's state, hook authority, and blocked reason rather than rebroadcasting them under the new label.
- `pane.report_agent_session_cwd` identifies the session by the payload's cwd instead of a pane id — the muse arm's report, whose reporter cannot carry `PAR_MUX_PANE_ID` (see [Agent-State Integrations](#agent-state-integrations)); the endpoint files it for the pane it is bound to, and a report with no matching pane is an error reply the reporter ignores.
- `pane.report_agent_telemetry` carries one `telemetry` object — versioned (`version`, currently 1), bounded (strings capped: `model` 128, `effort` 32; percents 0-100 rounded; no control characters), and stamped (`sampled_at_unix_ms` plus its own `source`, so clients can tell hook-reported from daemon-probed data).
  The shape mirrors the hub's normalized telemetry (par-remote-herd `status_telemetry.py`), and the producer is a hook tailing the agent harness's own status file — par-mux never parses transcripts.
  Malformed payloads are error-replied per the rules above, and a `sampled_at_unix_ms` in the future is error-replied the same way (it would saturate the freshness check to "maximally fresh" forever, QA-156).
  Four silent drops, each an ok no-op with nothing written: a sample older than the 55-minute freshness window (the hub's `STATUSLINE_FRESHNESS_SECONDS`, so daemon and hub age data out together — absent beats stale), a report at or below the last accepted `seq`, a sample older than the one already stored (a backward step, whatever its `seq`), and a report whose `agent` is not the pane's current label — telemetry attaches to a claim, it never takes one over.
  Telemetry is **display-only and ephemeral**: it never touches pane state or roster membership, never persists (the save format copies named identity keys only), and is cleared with the claim by `pane.release_agent` and the liveness sweep.
  An accepted write broadcasts `%agent-telemetry-changed` (identity only) and serves from the next `list-agents` as the row's trailing `telemetry=<base64>` token.
- `pane.release_agent` is the agent's exit announcement (herdr's SessionEnd shape): it clears the pane's whole claim — label, state, hook authority, blocked reason, sequence stamps, session identity, and telemetry — broadcasts `%agent-released`, and thereby returns the pane to the roster-less majority (a later restart respawns the original command, not a resume invocation for a dead session). Guards: the releasing `agent` must match the pane's current label, and the report must clear the same per-source `seq` rule; a failing guard is a silent ok no-op, like a stale report.
- Replies are one JSON line: `{"id":…,"result":"ok"}` or an error object.
- Authorization is the socket's own owner-only boundary: a hook claiming another pane's id is same-user by construction — the same trust model tmux control mode has. There is no per-connection authentication beyond the socket.

## Host Telemetry Probe

What the daemon can measure about a rostered pane's cwd that no hook can (card 01a0e3f1205371619309073eb5f803d6): disk-free percent (`statvfs` on Unix; Windows serves nothing for it — a field that cannot be measured is absent, never zero) and git state (`symbolic-ref` for the branch, `diff-index` + `ls-files --others` for dirty; a failing repo serves no verdict, never "dirty"), and the pane's `project` — the basename of the git work tree containing the cwd (a linked worktree names its main repo; outside a repo, the cwd's own name), found by walking up to a `.git` entry and reading at most a worktree's one-line `.git` file, no git run (pinned by `project_name_is_the_repo_root_basename`). The shapes mirror HerdDeck's hub probe (par-remote-herd `status_telemetry.py` `_probe_cwd`).

A dedicated thread sweeps every 30 s (`crates/par-mux/src/mux/host_probe.rs`), whole-sweep deadline 10 s, one git deadline 5 s — never on the roster poll, so `list-agents` costs zero probe syscalls and reads only what the sweep already wrote (the invariant the hub keeps by snapshot cadence). Results land in pane metadata as one canonical object with per-field `sampled_at_unix_ms` and `source: "host-probe"`, served as the roster row's `host_telemetry=<base64>` sibling token, aged per field against the same 55-minute window as hook telemetry — a stopped sweep fades the token out field by field. Like hook telemetry it is display-only (never persisted) and clears with the claim. It broadcasts nothing: `%agent-telemetry-changed` stays the hook's signal, and host fields refresh on the roster poll's next read.

## Agent Scrape Tier

Panes whose agent reports no state hook (claude, codex, grok, muse, droid, copilot, cursor, qwen, qodercli, devin) get state read from rendered content — OSC title and screen text matched against per-agent pattern rules bundled from herdr (Apache-2.0, credited per file). The scrape runs on the accept loop's idle poll once per second; there is no dedicated thread.

Structural rules (`crates/par-mux/src/mux/scrape.rs`):

- **Hooks stay primary.** A pane that has ever accepted a hook state report is hook-authoritative until it dies; the tick skips it outright, and a quiet hook agent holds its last claim — except the liveness sweep below, which clears the one thing a quiet hook can never correct: a dead agent.
- **A scrape never invents state.** An unmatched scrape clears its own earlier guess and yields nothing — never `idle` — with one opt-in exception: a set declaring `idle_fallback = true` (droid, copilot, cursor, whose CLIs draw no idle marker) reads an unmatched screen as `idle` (rule `idle_fallback`) while the process table proves the agent runs in the pane (a process in its tree whose argv names the label or the set's id), herdr's known-agent idle default. No proof (the agent's process gone, or Windows, which has no process table) keeps the pane unknown.
- **Provenance rides every surface**: the `source=` token on `%agent-state-changed` (followed by the owning `ws=` workspace token when the pane has one), the `<source>` column in `list-agents`, and the `agent_state_rule` metadata (which pattern fired).
- **`contains` matches case-insensitively** (herdr parity): needles lowercase at compile and the region text at match, so real-cased agent chrome ("Do you want to proceed?") hits rules authored in lowercase. `regex`/`line_regex` stay case-sensitive as written — make them `(?i)` explicitly when they must fold.
- **Every pattern list is ALL-of** (herdr parity): a `contains`, `regex`, or `line_regex` list holds when every entry matches (`line_regex` per pattern needs at least one matching line, not one line matching all). A rule carrying two patterns where only one is present does not fire.

Patterns ship bundled (`include_str!` from `crates/par-mux/src/mux/patterns/{claude,codex,grok,muse,droid,copilot,cursor,kimi,qwen,qodercli,devin,gemini,amp}.toml`; copilot also answers to `github-copilot`/`ghcs`, cursor to `cursor-agent`). gemini and amp are pattern-only ports (herdr ships no hook installer for them); mastracode has no upstream manifest. A local override file shadows the bundled set for its agent:

```text
<state_dir>/par-mux/agent-patterns/<agent>.toml
```

An override that fails to parse or validate falls back to the bundled set with a warning; a file for an agent with no bundled set adds it. `agent-explain -t %N` names which file a pane's agent is matched against and which rule matches its live screen (see the agent lifecycle bullet in [Command Reference](#command-reference)). Pattern files document which herdr rule regions are implemented (`osc_title`, `whole`, `bottom_non_empty_lines(N)`, `top_non_empty_lines(N)`).

### Claim liveness sweep

`pane.release_agent` only fires when the agent's hook is alive to send it. An agent that crashes or is killed would hold its roster entry and resume identity until relabel or pane death, so the same once-per-second tick runs a liveness sweep over hook-authoritative panes (`crates/par-mux/src/mux/foreground.rs`): a claim is stale when **no process descending from the pane's child matches the claimed agent's CLI** — the descendant tree, not the foreground process, because an agent backgrounded with Ctrl+Z or hidden behind an editor is still alive. The CLI match accepts the label as a path component or a `-`/`_`/`.`-suffixed variant (`pi`, `…/bin/pi`, `…/claude-cli/cli.js`, `pi.js`), which covers npm/bun-installed agents whose argv is an interpreter plus a package script. <!-- doc-path: example -->

Two mismatching ticks (not interrupted by a proven match) clear the whole claim and broadcast `%agent-released`, exactly as a release report would. The verdict is conservative in every ambiguous direction: a probe that goes blind (unreadable argv — the fresh-spawn window where macOS cannot read a mid-`exec` argv) keeps its recorded misses and never adds one, and Windows has no portable process-tree/argv access, so the sweep is inert there and hook release remains the only claim-clearing path.

One constraint follows from the argv match: an agent launched through a STAND-IN program whose argv carries no agent-name component — pi's or omp's `-x <program>` runs the named program in the pane instead of the agent CLI — can never match, so its claim is released after the two mismatching ticks however healthy the stand-in is. Name the stand-in after its agent and the sweep holds the claim: `pi -x 'sh -c "exec -a pi <program>"'` sets argv[0] to `pi`, which the label match takes verbatim.

## Self-Update and Installers

`par-mux update [--channel stable|preview] [--check]` uses the shared checksum-required engine described in [Binary Updates](#binary-updates). Stable selects the latest release, while preview selects the newest prerelease and falls back to stable when none exists. The archive SHA-256 must match the exact filename's entry in the release checksum list before extraction or replacement. The old image is retained as `<executable>.previous`. Restart uses the same-socket stop-and-restore handoff (D3.5), not live PTY transfer (D10). `--check` reports only. Missing assets, missing checksums, and corrupt downloads fail closed. The network uses `curl`, with HTTPS-only redirects, timeouts, and response size limits. `--machine` refuses `update` because updates manage the machine they run on.

The installer paths: `curl -fsSL https://raw.githubusercontent.com/paulrobello/par-term-emu-core-rust/main/scripts/install-par-mux.sh | sh` installs the latest release into `~/.local/bin` (override `PAR_MUX_INSTALL_DIR`), verifying the archive against the release's `SHA256SUMS` when `sha256sum` exists. The Homebrew tap flow (`brew tap paulrobello/par-mux && brew install par-mux`) is published from the formula template and steps in [`packaging/homebrew/README.md`](../packaging/homebrew/README.md) — filling its placeholders and pushing the tap is a release-cut action.

## Plugin System

A plugin is a directory with a `par-mux-plugin.toml` manifest whose hooks run shell commands when the daemon emits matching control notifications. Plugins live under `<state-dir>/par-mux/plugins/<name>/`; the daemon loads them at startup and `reload-config` re-reads the directory (an install or removal takes effect without a restart).

```toml
name = "bell"
version = "1"
description = "desktop notification on agent state changes"

[[hook]]
event = "agent-state-changed"
command = "notify-send par-mux \"$PAR_MUX_EVENT_LINE\""
timeout-ms = 3000
```

**The trust gate**: `par-mux plugin install <dir-or-git-url>` first resolves the source (a local directory as-is; a git URL through a depth-1 clone, manifest at the root or one level down) and prints the preview — the source, the plugin's identity, and EVERY command each event would run with its effective timeout. Without `--trust` that is all that happens; with it the directory is copied into `plugins/`, an entry lands in `plugins/installed.json` (name, version, source, install time), and the output names the directory. `plugin list` / `update <name>` (re-fetch from the recorded source, preview and `--trust` again) / `remove <name>` complete the lifecycle. None of it contacts a daemon.

**Hook execution**: dispatch rides the notification broadcast itself — a plugin sees exactly the line a client sees, from the same emitter, so plugin events can never drift from the wire. The hookable events are the stable slice: `session-changed`, `session-renamed`, `sessions-changed`, `window-add`, `window-close`, `window-renamed`, `layout-change`, `pane-exited`, `pane-respawned`, `pane-title-changed`, `agent-state-changed`, `agent-released`, `notification`, `config-changed`. The command line runs through POSIX `sh -c` (`cmd /d /c` on Windows) with `PAR_MUX_EVENT` (the event name) and `PAR_MUX_EVENT_LINE` (the emitted line) in its environment — and `PAR_MUX_SOCKET` deliberately REMOVED, so a plugin cannot drive the daemon's panes from the hook environment. Hooks run detached (stdin/stdout/stderr null), are killed at their timeout (default 3 s, 1..=60000), and at most 8 run at once daemon-wide — a flooding or stuck plugin drops its own events with a warning while the daemon keeps serving. Manifests that fail validation (unknown event, empty command, out-of-range timeout, name not matching the directory) are skipped at load with a warning, never failing the daemon.

## Saved Machines and the SSH Thin Client

`par-mux machine add <ssh-target> [--label L] [--session S]` saves a remote host profile under `[machines.<label>]` in the config file (`machine list` / `rename --to` / `enable` / `disable` / `remove` manage it; the edits are surgical, preserving every comment and neighboring key). The profile holds only a label, the ssh target, an optional default remote session, and an enabled flag — **no credentials, ever**: ssh itself owns authentication through the user's config, agent, and askpass, exactly as herdr's machines do. A digit-leading label rides TOML's quoted-key form (`[machines."10.0.0.4"]`) — the same label when parsed back.

`par-mux --machine <label> …` relays the invocation to that host: the command line is rebuilt as `ssh <target> par-mux <args>` with each argument quoted for the remote login shell, `--machine` itself stripped, and ssh's exit code returned. One-shot client verbs (`--cmd`, `status`, `ls`, `observe`, `control`, `--stop`) run under `BatchMode=yes` — a scripted relay fails fast instead of hanging on a prompt. `--machine <label> attach` is the **thin client**: it forces `--mode passthrough` (an explicit `--mode` wins) under `ssh -tt`, so the remote passthrough client drives the remote terminal and ssh owns the local terminal's raw mode; the local render TUI does not cross ssh in this form. Local-machine verbs (`--gen-config`, the agent-hook/extension/skill installers and status flags, `--foreground`, `--restart`, and `machine` itself) refuse the relay with the reason. Remote scripting needs no TUI at all: `par-mux --machine lab --cmd 'list-sessions'` answers over one ssh round trip.

Removing or disabling a machine leaves its remote sessions running — the profile is a pointer, not a lease.

## Persistence and Restart

State saving is crash-safe by construction (`crates/par-mux/src/mux/persist.rs`): serialize to `<file>.tmp`, fsync, rename over the target. The file is created `0600` on Unix (set at open, so there is no readable window between create and chmod) — the same owner-only posture as the socket. On Windows it inherits the ACL of the per-user state directory.

- **What persists:** the full tree (sessions, windows, panes, layout), each session's environment (`env`; a file written before the field existed loads with an empty one), each pane's screen and scrollback content, the paste buffer, each pane's user title (`user_title`, from `select-pane -T` — a save file written before the field existed loads with no title), each agent pane's session identity (`agent_session`: agent label, session id and/or transcript path, source tag, and the hook-reported resume argv) plus its last-known agent state and blocked reason (absent on files written before they existed, so older files restore exactly as before), and whether a pane was held dead at save time (`dead`, with its `exit_code` when one was recorded, ARC-114; both are skipped when unset, and a save file without them loads every pane alive, so older files restore exactly as before and older daemons ignore the fields).
  The workspace level persists too: each workspace's name, ordered session membership, and active-session index, plus the daemon's active-workspace pointer, and a worktree workspace's `worktree` link (`repo`, `checkout`, `branch`; absent for an ordinary workspace, so a file written before worktree workspaces existed loads with every workspace unlinked — see [Git worktree workspaces](#git-worktree-workspaces)).
  Format version is **3** — the version that introduced workspaces — and it is NOT backward compatible: workspaces are first-class, so a v2 (or older) state file is quarantined rather than migrated, and the daemon starts fresh.
  Persisted scrollback is capped per pane to the newest 100 000 cells (~1 250 lines at 80 cols) — a persistence bound only: the running pane keeps its full in-memory history, and the cap is what keeps a scrollback-maxed shutdown save from stalling daemon exit (measured 2026-09-22: uncapped, one flooded pane's final save was a 136 MB write holding SIGTERM exit for two minutes).
- **What deliberately does not persist:** the ordering `seq`, and `session_start_source`. A restored pane reports its own sequence from scratch, and start source describes the previous process's start. A scrape-tier pane's `agent` label does not persist either: the restore re-derives it from the respawn command's first token under the same scrape-pattern gate `adopt-pane` applies, so an adopted pane stays on the roster across a restart. The agent's last-known *state*, its provenance, and the blocked reason DO persist: the restored pane stays on the roster (`list-agents`, the sidebar) before its agent's next hook report. Staleness is bounded — the liveness sweep clears a claim whose agent is provably out of the pane's process tree, and the agent's next accepted report rewrites the state.
- **When it saves:** after every *successful mutating* command (the structural set: `new-session`, `rename-session`, `kill-session`, `set-environment`, `new-window`, `kill-window`, `rename-window`, `select-window`, `split-window`, `select-pane`, `resize-pane`, `swap-pane`, `break-pane`, `join-pane`, `move-window`, `swap-window`, `respawn-pane`, `kill-pane`, `set-buffer`, `clear-history`, `new-workspace`, `select-workspace`, `rename-workspace`, `kill-workspace`, `worktree-create`, `worktree-open`, `worktree-remove`, `set-workspace-respawn` with a value, `switch-client -t` (the displayed-session pointer), `agent-start` (the agent label it tags), `adopt-pane` (the agent label it tags), and `refresh-client -C`). Content commands (`send-keys`, `capture-pane`, `paste-buffer`, the lists) do not save — their staleness window is bounded by the next structural save and the clean-shutdown save.
- **Quarantine:** a corrupt or unknown-version state file is renamed aside (`<file>.quarantine-<timestamp>`) and the daemon starts fresh — unreadable state never blocks startup, and the evidence survives for inspection. A v1 or v2 (pre-workspace) state file is quarantined rather than partially read; workspaces shipped with no migration path by design.
- **Last-good snapshot:** every pane-bearing save also copies the state file to `<file>.lastgood`. Reap-driven saves (a pane's child exiting on its own) never touch the snapshot, and an empty final shutdown save leaves it alone — so the reboot/logout race, where the panes are killed before the daemon's own stop (shells turn SIGHUP into exit code 1, so the deaths look like ordinary child exits), cannot wipe the layout: the next start finds a pane-less state file and restores the snapshot instead. Two emptinesses are deliberate instead and clear the snapshot so the next start is fresh: an explicit `kill-pane` of the last pane (a Command save of an empty tree), and the daemon's exit-when-empty final save (`ShutdownEmpty` origin — every pane closed, no shutdown signal, whether or not clients are attached; see [Shutdown Semantics](#shutdown-semantics)). The accepted tradeoff that remains: a daemon stopped by SIGTERM whose last panes died by child exit just before the signal resurrects on the next start — the emptiness may be the logout race, and the snapshot must not be wiped on a guess. A corrupt snapshot is ignored, not quarantined; it is a derived cache, not evidence.
- **Snapshot history (D8):** every pane-bearing save at least 15 minutes after the newest kept snapshot also copies the state file into `snapshots/` beside it (`<stem>-<unix_ms>.state.json`, newest last, capped at 48 — herdr's `session-snapshots/` numbers). The copies are persistence's third recovery tier: an unreadable state file restores through `.lastgood`, then the newest snapshot (the quarantined evidence still lands beside them for inspection), and a raced empty state (the reboot race with `.lastgood` lost) restores through the tier the same way. A deliberate empty save clears the history with `.lastgood` — the fresh-start contract above extends to every recovery tier. Reap-driven saves keep both tiers. A snapshot failure warns and never fails the save.
- **Host shutdown (D9):** on Linux, a daemon built with the `shutdown-inhibit` cargo feature holds a logind `shutdown` delay lock from start and, when login1 broadcasts `PrepareForShutdown`, pushes a Shutdown save to the persist worker before the host terminates the pane children. logind caps the delay itself (`InhibitDelayMaxSec`, default 5 s), so a stuck save costs the window, never the shutdown. The feature is off by default (zbus is a heavy tree; the Linux CI budget is tight — the trade is recorded in MUX_DECISIONS D9); without the feature, on other platforms, or with no system bus reachable, the daemon serves exactly as before.
- **Live handoff (D10, design note):** replacing the daemon without losing live panes is designed (PTY-master fd transfer over the control socket via SCM_RIGHTS plus the tree capture, old daemon exiting without a final save once the new one acks) but deliberately not built: `par-mux --restart` is a stop-and-restore handoff that already preserves every pane's screen, scrollback, cwd, and agent identity (D3.5), losing only in-flight PTY output since the last save. The full design is recorded in MUX_DECISIONS D10 for when pane-survival across an update becomes a requirement.
- **Restore:** layout and content are rebuilt, and every live pane's process is new — the original processes died with the previous server.
  A pane that was **held dead at save time** (`dead: true`, ARC-114) is the exception: with `[daemon] remain-on-exit` on it is restored held dead, with no process spawned for it (with the default off, a persisted dead pane is dropped at restore through the kill-pane cascade — a window whose last pane was dead closes, and an emptied session and workspace go with it — except a pane that is, or after the earlier drops becomes, its workspace's last in a workspace with `set-workspace-respawn` on, which is restored held dead as the reaper's keep-one-tab opt-in would have held it; pinned by `auto_remove_restore_keeps_a_respawn_workspaces_last_dead_pane_held`, `auto_remove_restore_cascades_a_workspaces_last_dead_pane`, and `auto_remove_restore_drops_dead_entries_down_to_the_last`).
  It comes back as it stood: its exit code, its frozen screen and scrollback exactly as the client last saw them (the saved alternate screen and input modes are kept, not reset, because there is no new process to protect), its spawn command and agent identity.
  So `pane-info` reports `exited=<code>`, registration replays its `%pane-exited`, and `respawn-pane` (no `-k` needed) restarts it in place.
  Restore runs no agent resume invocation and no cwd spawn for a dead pane, and its persisted cwd is kept for the later `respawn-pane`.
  The OSC 7 hostname that came with that cwd (`cwd_host`, recorded only when the cwd was an OSC 7 report, skipped when absent, so older files load without it) is re-seeded beside it, so a remote report from an SSH session is still remote after the restart and never picks the respawn directory (SEC-128).
  The rest of this bullet describes a **live** pane's restore. A pane's terminal is rebuilt via `restore_for_new_process`, which keeps the saved screen content and scrollback but drops the state the *old* process left behind — as if the new process inherited a bare terminal, not the old one's TUI mode: it lands on the main screen (alternate grid cleared) regardless of what was active at save time, the scroll region and margins are reset, and cursor-key/keypad/mouse/focus/keyboard-protocol modes are off.
  This matters for a pane saved mid full-screen-app (htop, top): the pane returns to the shell's screen with its history intact, instead of resuming on a frozen, history-less alternate screen under a shell that never asked for it.
  Each pane's process respawns in its persisted cwd — captured at save from the shell's OSC 7 report, else the child's live process cwd — so a resumed pane (and a resumed agent, whose transcript lookup is cwd-sensitive) re-lands where it left off rather than in the daemon's start directory.
  A persisted cwd that no longer exists never fails the restore: the pane spawns in `$HOME` and a `par-mux: <dir> is gone` line is written into the restored pane.
  Agent panes respawn through their resume invocation when one can be built: the hook-reported `session_resume_argv` verbatim, else a per-agent table shipped in the binary (claude/codex/grok/muse/pi/omp, each in its own CLI's argument shape — `claude --resume <id>`, `codex resume <id>`, `grok --resume <id>`, `muse resume <id>` falling back to the workspace-scoped `muse resume --last` when no id was ever reported, `pi --session <id-or-path>`, `omp --resume=<id-or-path>`).
  The resume invocation is never persisted — a stored argv would freeze a vendor CLI shape into a state file newer code later restores.
  Failure splits by where it happens. A chain that cannot *build* an invocation (missing table entry, a session ref the table cannot use) falls back structurally to the pane's original spawn command — the pane spawns as it was.
  A chain that builds but *runs* into failure (uninstalled binary, a session id the CLI rejects) — or that runs to a clean exit — does not delete the pane: on Unix the invocation spawns with a surviving tail (`; { …; exec "$SHELL" }`), so ANY exit drops the pane onto a live shell carrying the restored screen, scrollback, and agent identity — an explanatory line names which exit happened (`agent resume failed` / `agent exited`), the reaper never sees a dead pane, and the identity survives for a retry or an explicit `kill-pane` (a clean exit keeps the pane: pinned by `a_clean_agent_exit_also_leaves_a_live_pane`).
  On Windows the resume argv never crosses a shell string: an `argv[0]` that resolves to a PE image (`.exe`/`.com`) spawns directly with CreateProcess-exact arguments, while `.cmd`/`.bat` shims (the npm-installed agent CLI shape — npm also drops an extensionless POSIX sh shim beside them, which CreateProcess cannot run and is never picked) and unresolved names run through a self-deleting bridge batch under `%COMSPEC% /d /c`, where double-quoted arguments carry spaces and `&` verbatim (`crates/par-mux/src/mux/win_resume.rs`; `%VAR%` expansion and embedded double quotes remain unrepresentable on a cmd line).
  A failed Windows resume still has no fallback shell — a non-zero exit still exits into the reaper, the one asymmetry that remains.
- **State is keyed by socket basename:** the state file is `<state_dir>/par-mux/<socket-stem>.state.json`, so a daemon started on a socket path with the same basename restores that stem's tree — this is the resurrection feature, and it spans daemon generations. Ephemeral and CI harnesses that start a fresh daemon per run on a stable socket path therefore inherit every prior run's panes, and the state file grows across runs (measured 2026-09-28 on a test harness: 94 panes, a 262 MB state file, daemon startup >10 s). Pass a distinct `--state-dir` per run (or remove the state file between runs) so each daemon starts fresh.

## Pane Reaping

A pane whose child process exits on its own (as opposed to via `kill-pane`) is detected automatically and either HELD or REMOVED according to `[daemon] remain-on-exit` (default `false` — auto-remove; `true` preserves the older held-dead behavior).
The accept loop's idle tick runs a pass every 250 ms (`REAP_INTERVAL`, `crates/par-mux/src/mux/server/mod.rs`; the pass is `crates/par-mux/src/mux/server/reap.rs`): a pane whose process died (the PTY reader's `is_running` flipped false on EOF, confirmed against the OS child handle — on Windows ConPTY that flag never flips after an exit, so the OS check is what observes the death there) is marked dead, its exit code recorded while the child handle can still be asked, and `%pane-exited %N [code]` broadcast.
With the hold ON, the pane keeps its id, window, layout, and frozen screen — the state a client needs to show "Process exited (code N)" and offer a restart; `respawn-pane` restarts it in place.
With the hold OFF (the default), the same pass then REMOVES the pane through the kill-pane contract: a surviving window gets `%layout-change` + `%window-pane-changed`, an emptied window gets `%window-close`, and the emptied session cascades with `%sessions-changed` — `%pane-exited` queues AHEAD of those, so an observer sees the death before the geometry.
**Keep-one-tab, the opt-in:** Paul's rule (2026-10-18) is the default — a pane closes when its child exits, running the full kill-pane cascade, so a workspace's LAST pane closes its workspace with it and the view lands on the next one (none remaining ends the client after the daemon's exit-when-empty grace; pinned by `auto_remove_cascades_a_workspaces_last_pane_on_app_exit` and `render_mode_ctrl_d_on_the_workspaces_last_pane_cascades_and_ends`). A workspace that opts into keep-one-tab (`set-workspace-respawn on`) holds its LAST pane dead exactly as under remain-on-exit: its window, session, and workspace stay, only `%pane-exited` is broadcast (no `%window-close`, `%sessions-changed`, or `%workspaces-changed`), and the respawn restarts it (or `respawn-pane` manually; both attach modes show the exit cue and bind `C-b r`, see [Management chords](#management-chords---x-c-). The test runs per death under one tree lock, so two same-pass deaths in a two-pane respawn-on workspace remove the first and hold the second. Explicit `kill-pane`/`kill-window`/`kill-session`/`kill-workspace` keep their full cascade — deliberate destruction is not an app exit.

**Keep-one-tab respawn (per-workspace option, off by default):** with `set-workspace-respawn -t <workspace> on` (or the workspace menu's `respawn on exit`), the workspace's LAST pane is restarted in place instead of held dead, under either remain-on-exit setting. The reaper runs `respawn-pane` on it after broadcasting `%pane-exited`: same pane id, window, session, and workspace; the pane's stored command (a plain shell pane gets a fresh shell, an agent pane relaunches its command) in its last known cwd; `%pane-respawned` follows, and no removal notification is sent. A pane that lived under one second is held dead instead (a crash-loop guard: a command that exits at once would otherwise respawn every reap tick). The flag is part of the workspace's saved state, so it survives a daemon restart (pinned by `workspace_respawn_restarts_the_last_pane_in_place`, `workspace_respawn_holds_a_pane_that_dies_at_once`, `workspace_respawn_survives_a_daemon_restart` and `parses_set_workspace_respawn_values_and_query`).
`respawn-pane` on a removed pane answers "no such pane" (the pane is gone, not held).
Pinned by `the_auto_remove_default_deletes_a_dead_pane`, `auto_remove_cascades_a_workspaces_last_pane_on_app_exit`, `auto_remove_removes_down_to_the_workspaces_last_pane`, and `kill_pane_of_a_workspaces_last_pane_still_cascades`. The code token is absent only when the OS had no exit status to report as the pass observed the death (the PTY closed before the child was reapable); a death by signal reports `1`.
`%pane-exited` is pushed once; a client that attaches after the exit reads it from `pane-info`'s `exited=` token — but only for a held pane, since a removed one leaves nothing behind to read.
At registration each control client is sent one `%pane-exited %N [code]` line per pane already held dead (with held-state replay, ENH-037), ahead of the registering command's reply block.
A death the reaper observes after the registration arrives only through the ordinary broadcast, so each client still learns of every death exactly once.
A client that wants the held panes' lines outside registration or a live death pulls them on demand with `pane-exited-replay` (ENH-042), delivered to the issuing client only.

`respawn-pane -t %N [-k] [-c dir] [--] [command]` restarts the pane's process in place: same pane id, window, and layout; a fresh terminal at the pane's current grid size; the old pane's user title carried over.
The flags (`-t`, `-c`, `-k`, in any order) must precede the command: flag parsing stops at the first word that is not one of them, or at `--`, so a `-k` or `-c` inside the command belongs to the command (`respawn-pane -t %0 sh -c 'sort -k 1 f'` neither kills nor changes directory).
A command that itself starts with `-` must follow `--`, and an unknown leading flag is an error. The command text is passed verbatim to the default shell (`$SHELL -c <command>`, or `cmd.exe /C <command>` on Windows), quoting and whitespace included.
The command and cwd come from the trailing text and `-c` when given, else the pane's stored spawn command and a default cwd: the shell's OSC 7 report only when its host is this machine (absent, `localhost`, or the local hostname, full or short form) and the directory exists here, else the live child's kernel-reported cwd (the `-k` case; a held dead pane has none), else the daemon's default pane cwd (its `$HOME` when none is configured).
A remote host's OSC 7 (an SSH session) never picks the directory (SEC-128). A still-running pane refuses without `-k` ("still running") and restarts with it, killing the old process first; a dead pane restarts with no flag.
`%pane-respawned %N` is the cue clients clear their exited-state chrome on. A respawned pane persists as the new process.
A pane still held dead when the tree is saved persists as dead (ARC-114): a save/restore cycle restores it held dead — when `[daemon] remain-on-exit` is on at restore time (the default off drops it; a workspace with the `set-workspace-respawn` opt-in keeps its last dead pane) — with its exit code and frozen screen, rather than respawning it (see [Persistence and Restart](#persistence-and-restart)).

Held panes are for clients that might come back. A persisting daemon where every pane is dead AND no client is connected collects itself through the same exit-when-empty path as an empty tree (see [Shutdown Semantics](#shutdown-semantics)) — with nobody watching and nothing alive, the daemon exits and its final save restores on the next start with every pane still held dead, as it stood (ARC-114), where `respawn-pane` restarts any of them.

A structurally killed pane is reaped at kill time instead: `PtySession::kill` follows its signal escalation (SIGHUP → SIGKILL inside portable-pty) with a bounded wait, so a child that traps SIGHUP cannot linger as a zombie until the daemon exits. The kill also runs off the tree lock on a short-lived thread once the pane is removed from the tree — the ~200 ms SIGHUP-grace poll and the reap wait would otherwise stall every command for their duration (tmux kills from its child reaper, off the layout lock, for the same reason).

## Shutdown Semantics

- **SIGTERM**: the handler makes one atomic store on the server's per-instance shutdown flag. The accept loop notices on its idle tick, broadcasts `%exit` to every client, and a final save captures content that arrived since the last structural save.
- **Exit-when-empty** (tmux's `exit-empty`, persisting daemon only): a daemon whose tree is empty — zero sessions ACROSS ALL WORKSPACES (an empty workspace alone never keeps it alive — and once a workspace's last session dies the workspace itself is removed, so a bare `new-workspace` in an otherwise empty daemon only postpones the exit by the grace) or only dead panes with no client connected — held for a 5 s grace, exits through the ordinary shutdown path — `%exit`, final save — with no one asking.
  ATTACHED CLIENTS DO NOT HOLD AN EMPTY DAEMON: the last session dying under a client (an explicit kill, or by default an app exit — `render_mode_ctrl_d_on_the_workspaces_last_pane_cascades_and_ends` pins the client rung) ends the server after the grace, and the client learns through the broadcast `%exit` and detaches (tmux semantics). An attached client DOES hold an all-dead tree: held panes (remain-on-exit, or a respawn-on workspace's last pane under auto-remove) are what it shows the exit cue and respawns over (pinned by `a_dead_pane_is_held_announced_and_respawnable` and `workspace_respawn_restarts_the_last_pane_in_place`).
  The grace is what keeps the two races it could lose won instead: creating a session (or respawning a pane) resets the clock, and a logout's SIGTERM, which follows pane deaths within moments, requests the exit first so the final save keeps `Shutdown` semantics (last-good snapshot preserved for the reboot-race resurrection).
  An exit-when-empty save carries the `ShutdownEmpty` origin instead. An empty tree clears the snapshot: everything was closed deliberately, so the next start is fresh rather than a resurrection of panes whose processes the user watched exit.
  An all-dead tree still holds its panes (remain-on-exit on, or a respawn-on workspace's last pane with it off), so the save refreshes the snapshot and the next start restores them held dead (see [Pane Reaping](#pane-reaping)); an emptied tree exits-when-empty and the next start is fresh.
  A restored all-dead tree exits again after the same grace once no client is connected. Consequences of the semantics: a bare `par-mux <name>` started with nothing to restore exits after the grace, and a one-shot `--cmd` client that finds an empty daemon ends it one grace after the tree emptied — attaching to an empty daemon gives the client the grace to create a session, then `%exit`.
  An embedded `MuxServer::run()` (no persistence) never exits-when-empty — it serves until stopped. `[daemon] exit-empty = false` (live via `reload-config`) disables the behavior entirely — the daemon sits empty indefinitely; the 5 s grace and its race protections apply whenever it is on (the default).
- **`kill-server`**: the client-initiated equivalent of SIGTERM — raises the same shutdown flag, so it takes the identical path (reply out, accept loop exits, final save, `%exit` to clients). `par-mux --stop`/`--restart` (see [Command Line](#command-line)) send this and wait for the socket to stop accepting.
- **Ignored signals** (Unix): SIGHUP, SIGINT, SIGQUIT, SIGTSTP, and SIGPIPE are installed as ignored while serving, so a terminal hangup or stray Ctrl-C/Ctrl-Z cannot stop a daemon whose panes outlive any one terminal (tmux's server does the same). The auto-spawned daemon additionally starts in its own session with no controlling tty (`setsid` in `spawn_daemon`), so terminal-generated signals never reach it in the first place.
- **Accept faults**: EMFILE, ENFILE, and ECONNABORTED on accept back off for a second and keep serving (tmux's server pauses on ENFILE/EMFILE the same way), so a burst of connections cannot end the daemon. Any other listener fault is logged and takes the same final-save path as SIGTERM before exiting — an accept error never silently discards unsaved work. The fault raises the shutdown flag just as SIGTERM does, so the exit waits only for the in-flight save to drain (bounded by the persist worker's poll) — never on connected clients, whose handler threads each hold a persist-sender clone a silent client would never drop. Both the fault exit and the SIGTERM exit drain in-flight saves before the process ends.
- **SIGKILL**: skips all of this and loses the last window's worth of unsaved content — an accepted trade (the design's [D3.3](MUX_DECISIONS.md)), not a bug.
- **Socket removal**: every exit path — SIGTERM, `kill-server`, exit-when-empty, accept fault — removes the socket file after the final save (the marker file on Windows), so `ls par-mux-*.sock` lists only live daemons. SIGKILL and a crash skip it, leaving a stale remnant the next bind reclaims.
- Each `MuxServer` instance has its own shutdown flag; stopping one does not affect another server in the same process.

## Streaming Panes to the Web

`par-term-streamer --mux-socket <path>` (built with `--features streaming-bin,mux`) serves the daemon's panes to the web and mobile frontend: each streaming session mirrors one pane (`?session=pane-N`), keys and mouse reach it through `send-keys -H`, and a viewer's resize is recorded as the streaming server connection's sizing contribution under the [smallest-attached-client rule](#command-reference) (opening a viewer never resizes the pane). A graphic a pane app placed (OSC 1337, Sixel, Kitty) streams to viewers as a pane-tagged `inline_image` frame. The library form is `streaming::MuxSessionFactory`. Full behavior, including the output-only input semantic for pane images: [STREAMING.md — Inline Images](STREAMING.md#inline-images).

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

## macOS App Bundle (TCC Permissions and Autostart)

Panes under the par-mux daemon attribute their macOS permissions to the daemon's responsible process. A bare CLI daemon has no grantable identity, so TCC silently blocks screen capture, microphone, and local-network sockets — no prompt ever appears (the multiplexer-daemon variant of macOS Local Network privacy). The fix is an LSUIElement wrapper bundle whose launcher fires the TCC prompts and then execs the daemon in-process, so every pane child inherits the bundle's grants. Attribution was verified to survive par-mux's daemonize-to-background in the herdr wrapper this is ported from.

**Build** (macOS only): `make app` compiles and adhoc-signs `build/Par-mux.app` from `crates/par-mux/app/` (`launcher.c`, `permissions.m`, `Info.plist`) and lints the plist. It deliberately does not install. Install once by hand:

```bash
cp -R build/Par-mux.app /Applications/
```

**Never modify or re-sign a granted bundle.** Every TCC grant binds to the bundle's adhoc signature — rebuilding over `/Applications/Par-mux.app` (or any content change plus re-sign) can void the grants. Change the wrapper by deleting the old bundle, installing the new one, and granting again. Source edits are always safe: `make app` only writes `build/`, never `/Applications`.

**What the launcher fires** on every launch, each preflighted (silent once granted) and presented/owned by the system, so the prompts survive the immediate exec into the daemon:

- Screen Recording — `CGRequestScreenCaptureAccess` (no Info.plist usage key exists; the API call is the trigger)
- Accessibility — `AXIsProcessTrustedWithOptions` with the prompt option
- Microphone — `AVCaptureDevice requestAccessForMediaType:` (plist `NSMicrophoneUsageDescription`)
- Local Network — one short TCP connect to the default gateway (no preflight API exists; a bare UDP datagram does not trip the check — only a connect does; plist `NSLocalNetworkUsageDescription`)

No prompt API exists for Full Disk Access, Developer Tools, or per-target Automation — grant those by hand in System Settings → Privacy & Security when a pane needs them.

Bare launch runs `par-mux --foreground` (the launchd shape: the launcher process becomes the daemon — no daemonize fork — so launchd supervises it and `KeepAlive` restarts only a crash; a clean `par-mux stop` exits 0 and stays down; no terminal means no auto-attach). Arguments are forwarded verbatim, so the bundle binary is also a faithful par-mux client proxy; the prompt-trigger test that never disturbs a running daemon is:

```bash
/Applications/Par-mux.app/Contents/MacOS/Par-mux status
```

The launcher execs `$PAR_MUX_BIN` if set, else `~/.cargo/bin/par-mux` (the `make par-mux-install` location), else `/usr/local/bin/par-mux`. Its stdout/stderr append to `~/Library/Logs/par-mux/daemon.log`.

**Autostart (launchd)**: `crates/par-mux/app/` ships two LaunchAgent templates (`__HOME__` placeholder to expand at install). Wiring the daemon through the bundle replaces any bare-daemon autostart:

```bash
mkdir -p ~/Library/Logs/par-mux
sed "s|__HOME__|$HOME|g" crates/par-mux/app/par-mux-daemon.plist.template \
  > ~/Library/LaunchAgents/com.paulrobello.par-mux.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.paulrobello.par-mux.plist
```

If a daemon started outside the bundle is already running, stop it first (`par-mux stop` — it saves the tree, and the bundle-started daemon restores it), then `launchctl kickstart gui/$(id -u)/com.paulrobello.par-mux`.

Grants bind to the bundle, not the process, so every daemon started through the bundle keeps them — launchd boot, `KeepAlive` crash restart, reboot. A plain `par-mux restart` (or bare `par-mux`) from a terminal spawns the fresh daemon from the *terminal's* process tree: the grants still exist, but pane children lose bundle attribution until the next bundle start. Restart through the bundle instead:

```bash
launchctl kickstart -k gui/$(id -u)/com.paulrobello.par-mux
```

That also replaces the usual `par-mux --restart` upgrade flow — the launcher execs `~/.cargo/bin/par-mux` at start, so rebuild (`make par-mux-install`) then kickstart.

**Log capping**: launchd never rotates `StandardErrorPath` files and the daemon log only grows, so a second agent caps them hourly in place (50MB plus a single `.1.gz`). Truncate-in-place is safe because launchd and the launcher open their logs `O_APPEND`; rename-style rotation would strand writes in the moved inode:

```bash
install -m 755 crates/par-mux/app/logcap.sh ~/.local/bin/par-mux-logcap.sh
sed "s|__HOME__|$HOME|g" crates/par-mux/app/par-mux-logcap.plist.template \
  > ~/Library/LaunchAgents/com.paulrobello.par-mux-logcap.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.paulrobello.par-mux-logcap.plist
```

**End-to-end check** (after granting the prompts): start the daemon through the bundle, spawn a pane, and from that pane hit a LAN host with a non-Apple-signed binary (Apple-signed tools like `curl` are exempt and prove nothing). A dead target host mimics the privacy block exactly — ping the target first.

## Testing

The mux test suites must run serialized — several spawn daemons on default-style paths that collide under parallel execution. The integration tests that exec the daemon fail to compile without `mux-bin`:

```bash
cargo test -p par-mux --features attach -- --test-threads=1
```

The integration suites live in the `par-mux` workspace member, `crates/par-mux/tests/mux_*.rs` (`mux_daemon`, `mux_restart`, `mux_reattach`, `mux_hooks`, `mux_agents`, `mux_attach`, and others); `mux_feature_isolation` stays in the root `tests/` because it guards the root crate's `mux` feature gate.

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
| `crates/par-mux/src/mux/mod.rs` | Module root and re-exports |
| `crates/par-mux/src/mux/server/` | Socket server: `mod.rs` accept loop (`MuxServer`), persist worker, scrape heartbeat; `client.rs` per-client thread (`handle_client`, registration); `protocol.rs` bounded control-line reads and writes; `broadcast.rs` fan-out and reply sinks; `reap.rs` dead-pane reaping; `endpoint.rs` hook-only pane endpoints |
| `crates/par-mux/src/mux/dispatch/` | `mod.rs` `dispatch_command`, `Ctx`/`Outcome`, and the shared emit-and-save tail; per-command handlers (`cmd_<name>`) by group in `panes.rs`, `windows.rs`, `sessions.rs`, `client.rs`, `buffers.rs` |
| `crates/par-mux/src/mux/command/` | Control-command parsing: `mod.rs` holds the `COMMANDS` table, `MuxCommand`, the `mutates()` persistence rule, and the shared `Args` grammar; one `parse_<cmd>` per command lives in `panes.rs`, `windows.rs`, `sessions.rs`, `buffers.rs`, with the send-keys payload grammar in `keys.rs` |
| `crates/par-mux/src/mux/emit.rs` | `TmuxNotification` → wire lines, reply blocks, octal output escaping |
| `crates/par-mux/src/mux/tree/` | The workspace/session/window/pane tree and its operations (`mod.rs` types, accessors, and target resolvers — workspaces included; `lifecycle.rs` create/respawn/kill for all levels; `layout_ops.rs` splits, moves, resizes, and the `mutate_layout` choke point) |
| `crates/par-mux/src/mux/layout.rs` | The binary `LayoutTree` split geometry and tmux layout strings |
| `crates/par-mux/src/mux/pane.rs` | `MuxPane` (PTY + `Terminal`), `PaneFactory`, `ShellPaneFactory`, the env contract |
| `crates/par-mux/src/mux/ids.rs` | `WorkspaceId`/`SessionId`/`WindowId`/`PaneId` (`+N`/`$N`/`@N`/`%N`) and id allocation |
| `crates/par-mux/src/mux/ipc.rs` | Cross-platform local socket transport (Unix socket / Windows named pipe), `0600`/DACL binding, stale-path reclamation |
| `crates/par-mux/src/mux/discovery.rs` | Server registry (`<state-base>/servers/<stem>.json`) and `--list-servers` enumeration: register/unregister, the named-default directory sweep, and the `version`/`list-sessions` probe |
| `crates/par-mux/src/mux/persist.rs` | Save format (version 3, workspaces included), atomic writes, quarantine, restore incl. the agent resume path |
| `crates/par-mux/src/mux/host_probe.rs` | The host telemetry probe: disk + git per pane cwd, its 30 s cadence thread, and per-field freshness serving |
| `crates/par-mux/src/mux/hooks/` | The JSON hook-report grammar: `pane.report_agent`, `pane.report_agent_session`, `pane.report_agent_telemetry`, `pane.release_agent` (`mod.rs` dispatch + shared header/seq/reply helpers, `report.rs` state and session, `telemetry.rs`, `release.rs`) |
| `crates/par-mux/src/mux/scrape.rs` | The scrape tier engine and its 1 s tick, incl. the claim liveness sweep |
| `crates/par-mux/src/mux/foreground.rs` | Process-table snapshot + descendant-tree liveness probe for hook claims (macOS `sysctl KERN_PROC_ALL` + `KERN_PROCARGS2` — libproc is ancestry-gated; Linux `/proc`; inert on Windows) |
| `crates/par-mux/src/mux/agent_resume.rs` | The per-agent resume invocation table |
| `crates/par-mux/src/mux/win_resume.rs` | Windows resume transport: PE resolution and the cmd bridge batch |
| `crates/par-mux/src/mux/client.rs` | `MuxClient`: connect/attach, `send`, notifications, spawn/kill daemon |
| `crates/par-mux/src/mux/patterns/` | Bundled scrape patterns: `claude.toml`, `codex.toml`, `grok.toml`, `muse.toml`, `droid.toml`, `copilot.toml`, `cursor.toml`, `kimi.toml`, `qwen.toml`, `qodercli.toml`, `devin.toml`, `gemini.toml`, `amp.toml`. gemini and amp are PATTERN-ONLY (herdr ships no hook installer for them); mastracode has no upstream manifest; opencode is unported (herdr uses a JS plugin + TUI config edits, not the hook transport); qwen's `osc_tool_progress_working` rule is dropped pending a PaneSnapshot progress field |
| `crates/par-mux/src/bin/par_mux/main.rs` | The daemon binary: CLI, state load/quarantine, signal handlers, `--restart` detach |
