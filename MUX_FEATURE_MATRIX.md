# par-mux vs herdr — Feature Matrix

Feature comparison of **par-mux 0.62.0** (this repository, main @ `dacba503`) and **herdr 0.9.5** (`~/Repos/herdr`, master @ `143bf5fd`), recorded 2026-10-19.

- **par-mux** — the multiplexer shipped by this repo (`crates/par-mux/`): a daemon speaking the tmux control-mode protocol over a local socket, with PTY-backed panes, an attach TUI, and AI-agent awareness wired into the par-term ecosystem.
- **herdr** — a self-contained "terminal workspace manager for AI coding agents": one Rust binary, client/server, UI in each client, built on a vendored Ghostty VT core.

The two are siblings in the same ecosystem, not competitors. par-mux deliberately borrows from herdr: the herdr-style tab strip and border styles, outer-environment hygiene parity, and herdr's agent screen-scrape patterns bundled under Apache-2.0 with per-file credit (`crates/par-mux/src/mux/patterns/`).

Row claims cite source keys expanded in [Sources](#sources) — documentation by section name, code by file path. Versions above are the ones verified; features may differ in later releases.

## Contents

- [At a glance](#at-a-glance)
- [1. Architecture and process model](#1-architecture-and-process-model)
- [2. Session / workspace model](#2-session--workspace-model)
- [3. Pane operations](#3-pane-operations)
- [4. Client UI (attach TUI)](#4-client-ui-attach-tui)
- [5. Scripting and control](#5-scripting-and-control)
- [6. AI agent integration](#6-ai-agent-integration)
- [7. Hooks and notifications](#7-hooks-and-notifications)
- [8. Persistence](#8-persistence)
- [9. Configuration](#9-configuration)
- [10. Platform support](#10-platform-support)
- [11. Security](#11-security)
- [12. Command surface](#12-command-surface)
- [Where each leads](#where-each-leads)
- [Appendix: command enumerations](#appendix-command-enumerations)
- [Sources](#sources)

## At a glance

| | par-mux 0.62.0 | herdr 0.9.5 |
| --- | --- | --- |
| Self-description | tmux-control-mode multiplexer daemon with attach TUI | Terminal workspace manager for AI coding agents |
| Process model | Daemon owns panes; attach clients connect over one socket | Server owns processes; each client renders its own UI |
| Control protocol | tmux control-mode grammar + JSON agent-hook reports | JSON CLI and socket API with published JSON-Schema |
| Terminal core | par-term-emu-core (VT100–VT520, Sixel/iTerm2/Kitty) | Vendored Ghostty VT core (`crates/ghostty-vt`) |
| UI paradigm | Keyboard-first ratatui renderer, plus passthrough mode | Mouse-first ratatui UI; keyboard is an optional layer |
| Agent breadth | 4 hooked agents + 2 extension agents, scrape fallback | 22 supported integrations + 5 self-reporting agents |
| Remote access | Web/mobile mirror via `par-term-streamer --mux-socket` | SSH thin client (`--remote`), saved machines, `--machine` |
| Distribution | crates.io + per-platform release archives with SHA256SUMS | curl / Homebrew / mise / Nix / install.ps1, self-update channels |

## 1. Architecture and process model

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Client/server split | Daemon owns PTY panes in a tree; clients attach in place of tmux | Background server owns real terminals; clients attach to render | PM-MUX / H-CONCEPTS |
| Wire protocol | One line-based socket, two grammars classified by first byte: tmux control command vs JSON hook report | Local IPC over Unix sockets / Windows named pipes, JSON methods | PM-MUX / H-PLUG |
| Daemon lifecycle | Detaches by default (fork + setsid); `--foreground` to serve in-process; Windows always foreground | `herdr server` headless mode for supervised setups, virtual terminal size when no client | PM-MUX, PM-CLI / H-CLI |
| Multiple instances | Named default sockets (`par-mux-<name>.sock`); `--list-servers` registry with pointer files and dead-entry pruning | Named sessions are fully separate server namespaces with own sockets and state | PM-MUX / H-CONCEPTS |
| Simultaneous clients | Yes; broadcasts to registered clients; per-client 4096-line queues with eviction | Yes; each client can view a different workspace/tab; shared tabs follow the last-interacting client | PM-MUX / H-CONCEPTS |
| Nested-instance guard | Refuses serve mode inside a pane unless `PAR_MUX_ALLOW_NESTED=1` | Blocked by default unless `experimental.allow_nested` | PM-MUX / H-CFG |
| Resource hardening | Raises RLIMIT_NOFILE at startup; accept-fault backoff; bounded broadcast queues | Not documented at this level | PM-MUX / — |
| Remote access | `par-term-streamer` mirrors panes to web/mobile (view + input) | SSH thin client (`--remote`), saved machine profiles, `--machine` remote scripting without a TUI | PM-MUX / H-CLI |
| Embedding contract | Rust API: `MuxClient`, `MuxServer`, `PaneFactory` under the `mux` feature | Stable client endpoint contract (generation 1) with frozen codecs for plugins/clients | PM-MUX / H-AGENTS.md |

## 2. Session / workspace model

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Hierarchy | daemon > **workspace > session > window > pane**; ids `+N` `$N` `@N` `%N` | **session > workspace > tab > pane**; opaque stable ids `w1`, `w1:t1`, `w1:p1` | PM-MUX / H-CONCEPTS |
| Extra grouping level | Workspaces sit above sessions (tmux has none); each tracks its own active session | Sessions are separate server namespaces; workspaces are project-level containers | PM-MUX / H-CONCEPTS |
| Naming | Typed ids beat names; names match exactly; session names unique, duplicates refused | Opaque public ids never reused after close; rename supported | PM-MUX / H-SKILL |
| Default tree | Empty attach creates workspace+session `default`, window `1`, one pane | Launch/attach reattaches the persistent default session | PM-MUX / H-CONCEPTS |
| Git worktree workspaces | No worktree management (host probe only names the work tree) | `worktree create/open/remove`; worktree workspaces grouped under the parent repo workspace | PM-MUX / H-CLI |
| Layout portability | Binary `LayoutTree` emitting tmux layout strings | `layout.export` / `layout.apply` / `layout.set_split_ratio` socket methods | PM-SRC / H-API |
| Workspace metadata | Agent roster grouped by workspace (`ws=+N`) | Per-workspace reportable tokens (e.g. `$jj_status`) in sidebar space rows | PM-MUX / H-CFG |

## 3. Pane operations

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Split | `split-window -h/-v [-b] [-p 1-99] [-c dir]` | `pane split` with `--ratio` (float), `--cwd`, `--env`, `--focus/--no-focus` | PM-MUX / H-CLI |
| Resize | Directional cells (`-L/R/U/D`), absolute `-x/-y`, resize mode in attach | Directional `--amount FLOAT`, prefix resize mode, optional direct bindings | PM-MUX / H-CLI |
| Zoom | `resize-pane -Z`; session state, broadcast flag, unzoom on select-away | `pane zoom --toggle/--on/--off` (`prefix+z`) | PM-MUX / H-CLI |
| Swap / move | `swap-pane`, `break-pane`, `join-pane`, `move-window`, `swap-window` | `pane swap` (directional or explicit), `pane move` to tab / new tab / new workspace | PM-MUX / H-CLI |
| In-place respawn | `respawn-pane -t %N [-k]` keeps id/layout/title | No in-place respawn command | PM-MUX / — |
| Dead-pane handling | 250 ms reaper; `remain-on-exit`; `set-workspace-respawn` with crash-loop guard; held-dead state persisted | Close-confirmation option; no held-dead/respawn tier | PM-MUX / H-CFG |
| Cross-client sizing | Per-window minimum over displaying clients (tmux `window-size smallest`) | Shared tabs follow the last-interacting client | PM-MUX / H-CONCEPTS |
| Pane chrome | `refresh-client -C WxH -I border=,gap=,gutter=` per-pane chrome declaration | UI-level pane borders / gaps / outer borders options | PM-MUX / H-CFG |
| Navigation | `select-pane/window/workspace`, `switch-client -t`; prefix arrows/`o`/`n`/`p`/`(`/`)`/`W` | `pane focus --direction`, `pane neighbor`, `pane edges`; `prefix+h/j/k/l`, `prefix+tab` | PM-MUX / H-CLI, H-KEYS |

## 4. Client UI (attach TUI)

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Modes | `render` (ratatui TUI) and `passthrough` (host terminal is the VT emulator) | Single terminal UI mode per client | PM-MUX / H-CONCEPTS |
| Renderer | Per-pane core `Terminal` mirrors mapped to ratatui buffers; damage-diffed frames at 16 ms | ratatui 0.30 + crossterm 0.29, UI runs in each client | PM-MUX / H-README |
| Mouse | SGR + button-event capture; tab-strip click/context menus; wheel scroll; forwarding in the pane's negotiated dialect | Mouse-first: click focus, drag split borders, right-click menus, drag-select copy, Ctrl-hold link highlight | PM-MUX / H-CONCEPTS |
| Tab strip | Top strip, drag-to-reorder, `+` new-tab button, active tab pinned under truncation | Top/bottom placement, right-side status entries (zoom/hostname/datetime/command), hide-when-single-tab | PM-MUX / H-CFG |
| Sidebar / roster | Workspace side panel with agent roster (row presets, drag width, collapse rail, persisted) | Agent roster sidebar + space rows, `rows_by_agent` overrides, hide rules, sort, compact/hidden modes | PM-MUX / H-CFG |
| Settings UI | Tabbed settings dialog writing config keys via `toml_edit`, then live reload | `prefix+s` settings UI | PM-MUX / H-KEYS |
| Pickers / palette | Session/window picker, workspace picker, command palette (`:`) | Go To picker (`prefix+g`) with search and state filters | PM-MUX / H-KEYS |
| Help | Themed modal with `/` filter, categories | Live keybind help `prefix+?` with `/` filter | PM-MUX / H-KEYS |
| Copy / scroll mode | Viewport over client scrollback; `v`/`y` copy via OSC 52; `/` search with `n`/`N` | hjkl/w/b/e motions, `/`/`?` search, select/copy, edit scrollback in `$EDITOR` | PM-MUX / H-KEYS |
| Theming | Border styles (`herdr`/unicode/double/heavy/ascii), border colors, OSC 11 background probe | 11 built-in themes, auto light/dark switching, per-token overrides, accent color | PM-MUX / H-CFG |
| Graphics in UI | Kitty `t=t` temp-file media retained daemon-side for client mirrors | Kitty graphics rendered in panes (enabled by default) | PM-MUX / H-CFG |
| IME / CJK | Not found | ASCII input-source switch in prefix mode, CJK cursor reveal with agent allow-list | — / H-CFG |
| Sounds | No | Built-in per-event sounds, custom paths, per-agent mute | — / H-CFG |
| Popups | No | Popup command windows (`[[keys.command]] type = "popup"`) | — / H-CFG |
| Onboarding | No | First-run onboarding flow | — / H-CFG |
| Mobile layout | Via web frontend (streaming) | Single-column layout below a width threshold | PM-MUX / H-CFG |

## 5. Scripting and control

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| One-shot CLI | `par-mux --cmd '<tmux command>'`; framed reply, exit 0/1/2 | `herdr <noun> <verb>` returning JSON; CLI and socket share argument definitions | PM-MUX / H-CLI |
| Env contract | `PAR_MUX_PANE_ID`, `PAR_MUX_SOCKET`, `PAR_MUX_CONTROL_SOCKET`, `PAR_MUX_ENV`, `PAR_MUX_SESSION(_ID)`, `PAR_MUX_WINDOW_ID`, `PAR_MUX_BIN` | `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_BIN_PATH`, `HERDR_SOCKET_PATH`, `HERDR_CONFIG_PATH`, `HERDR_SESSION`, `HERDR_LOG`, … | PM-MUX / H-CLI |
| Self-targeting | Socket precedence `--socket` > name > `$PAR_MUX_SOCKET` > default; works from inside a pane | `--current` targets the calling pane; `pane split` defaults to the caller | PM-MUX / H-SKILL |
| Capture output | `capture-pane -S/-E` tmux offsets; `-e` for SGR-styled lines | `pane read` / `agent read` with sources `visible \| recent \| recent-unwrapped \| detection`, `--lines`, text/ANSI | PM-MUX / H-CLI |
| Send input | `send-keys` tmux contract (`-l` literal, `-H` hex, key re-encoding vs pane mode) | `pane send-text`, `pane send-keys` (combo syntax), `pane run` bracketed-paste-aware atomic submit | PM-MUX / H-CLI |
| Wait on output | Not found (poll `capture-pane`) | `pane wait-output (--match \| --regex) --source --lines --timeout` | — / H-CLI |
| Paste buffers | Single `default` slot: `set-buffer` / `show-buffer` / `paste-buffer` | No tmux-style buffers; copy-on-select and clipboard integration instead | PM-MUX / H-CFG |
| Environment scoping | `set-environment` per session (durable, persisted) | `pane split --env` per spawn | PM-MUX / H-CLI |
| Capability discovery | `list-commands` feature tokens; `client-snapshot` one-round-trip read | `herdr api schema` (JSON-Schema, bundled in docs); `herdr api snapshot` | PM-MUX / H-API |
| Event stream | `%`-notification broadcasts to registered clients; late-client replay of held state | `events.subscribe` / `events.wait`, live with no replay, `events_lost` backpressure signal | PM-MUX / H-API |
| Raw terminal streams | Not exposed directly (streaming server mirrors instead) | `terminal attach`, `terminal session control` (writable), `terminal session observe` (read-only, multiple observers) | PM-MUX / H-CLI |
| Shell completions | Not found | zsh, bash, fish, powershell, elvish | — / H-CLI |

## 6. AI agent integration

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Hooked / integrated agents | claude, codex, grok, muse hooks + pi/omp state extensions | 18 installable integrations; 22 agents supported + 5 self-reporting without install | PM-MUX / H-AGENTS |
| Detection tiers | Hooks primary; scrape tier from screen text + OSC title using bundled herdr patterns (local override dir) | Foreground process discovery, evidence-based screen manifests, vendor hooks; `agent explain` shows the matched rule | PM-MUX / H-AGENTS |
| States | working / blocked / idle (`unknown` never reported, `idle` never guessed) | blocked / working / done / idle / unknown (blocked deliberately strict; idle is never authorization) | PM-MUX / H-AGENTS |
| Structured telemetry | `report_agent_telemetry`: model, effort, context used/remaining %, reset times | Metadata reports with `--seq`/`--ttl-ms`; no model/context telemetry equivalent | PM-MUX / H-CLI |
| Claim liveness | Once-per-second descendant-tree probe clears stale claims (macOS/Linux) | Agent identity survives Ctrl+Z suspend / `fg` resume and follows pane moves | PM-MUX / H-SKILL |
| Resume after restart | Per-agent resume argv table; failed resume drops onto a live shell | Native conversation restore for 18 integrations, staggered startup, per-agent minimum versions | PM-MUX / H-STATE |
| Launching agents | `AgentPaneFactory` for Rust embedders | `agent start <name> --kind KIND --pane` into a ready shell; returns only when detected ready | PM-MUX / H-CLI |
| Prompting / orchestration | `send-keys` into the pane | `agent prompt --wait/--until/--timeout` with anti-stall gating; server-owned `agent.wait` | PM-MUX / H-CLI |
| Bundled skill | `--install-skill` writes the embedded skill to `~/.claude/skills/` | Skill bundled in the binary (`herdr --skill`) and installable via `npx skills add` | PM-MUX / H-SKILL |
| Roster query | `list-agents` with base64 `reason`/`telemetry`/`host_telemetry` tokens | `agent list` / `agent get` JSON | PM-MUX / H-CLI |
| Direct agent attach | `attach -t %N` to the pane | `agent attach [--takeover]`; `agent.view.set/clear` socket methods | PM-MUX / H-AGENTS |
| Detection rules update | Static bundle + local TOML overrides | Hot-reloadable manifests, remote-updated from herdr.dev, `server update-agent-manifests` | PM-MUX / H-AGENTS |

## 7. Hooks and notifications

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Delivery channels | `inside` (in-app toast + bell), `terminal` (OSC 99 under kitty / OSC 9 elsewhere), `system` (osascript/notify-send), `off` | `herdr` toasts, `terminal`, `system`; configurable delay and toast positions | PM-MUX / H-CFG |
| State filtering | `notify-states` (default blocked+idle), background-only rule | Per agent state and background-workspace triggers | PM-MUX / H-README |
| Click / jump to source | Not found | `open_notification_target` keybinding; Windows toast click focuses the pane | — / H-CFG |
| Ad-hoc notification command | No (system channel is the OS notifier path) | `herdr notification show <title> [--body] [--position] [--sound]` | PM-MUX / H-CLI |
| Plugin event hooks | No plugin system | Plugin manifest `[[events]] on = "worktree.created"` triggers; `[[link_handlers]]` for OSC 8 links | — / H-PLUG |
| OSC handling | OSC 11 background probe, OSC 22 drag-cursor shape, kitty `t=t` media retention | OSC 0/2 titles tracked server-side (sidebar tokens), OSC 8 clickable links | PM-MUX / H-AGENTS |
| Ad-hoc pane metadata | Not found | `pane report-metadata` sequenced, TTL-expiring, max 32 sources per pane | — / H-CLI |
| Host telemetry | Daemon-measured disk-free %, git branch/dirty, project name, 30 s sweep | Per-workspace metadata tokens (e.g. `$jj_status`) | PM-MUX / H-CFG |

## 8. Persistence

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Survives detach | Yes (daemon keeps panes) | Yes (server keeps processes; strongest path) | PM-MUX / H-STATE |
| Survives restart | Full tree restore; every live pane respawns in its persisted cwd; held-dead panes restored | Snapshot restore of workspaces/tabs/panes/cwd/layout/focus; processes do not survive | PM-MUX / H-STATE |
| Screen history | Persisted by default (newest 100 000 cells per pane) | Optional replay after full restart, off by default because output can contain secrets | PM-MUX / H-STATE |
| Crash safety | tmp file + fsync + rename; 0600 at open; `.lastgood` snapshot | Write-to-disk-before-replace; `session-backups/` keeps 3 recovery copies | PM-MUX / H-STATE |
| Snapshot history | Single state file per socket | Up to 48 layout snapshots in `session-snapshots/`, throttled to one per 15 minutes | PM-MUX / H-STATE |
| Format versioning | Format v3; v1/v2 files quarantined, daemon starts fresh | `session.json` + snapshots; no quarantine tier documented | PM-MUX / H-STATE |
| OS integration | None found | Linux logind shutdown inhibitor saves the session before host shutdown | — / H-STATE |
| Live handoff | No | Experimental `herdr update --handoff` transfers live pane PTYs to a replacement server | — / H-STATE |
| What is saved | Tree, session env, screens/scrollback, paste buffer, titles, agent session identity, dead-held state | Workspaces, tabs, panes, cwd, layout, focus (+ optional pane history) | PM-MUX / H-STATE |

## 9. Configuration

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| File | `<config dir>/par-mux/config.toml`; `$PAR_MUX_CONFIG` override | `~/.config/herdr/config.toml` (Windows `%APPDATA%`); `HERDR_CONFIG_PATH` override | PM-MUX / H-CFG |
| Surface size | ~40 `[client]` keys + `[daemon]` socket/state/endpoint keys | Theme, terminal, ~50 key actions + custom commands, server, worktrees, ui (+toast/sound), session, remote, experimental, advanced | PM-MUX / H-CFG |
| Live reload | `reload-config` command, `C-b C-r` chord, `%config-changed` broadcast to clients | `server reload-config`, `prefix+shift+r` | PM-MUX / H-CLI |
| Config generator | `--gen-config [--force]` writes effective values | `--default-config` prints the full annotated default | PM-MUX / H-CLI |
| Docs-sync gate | `mux-docs-check` in `make checkall` fails on drift | `scripts/config_reference_check.py` keeps docs in sync | PM-MUX / H-AGENTS.md |
| Per-client state | `client-state.json` (sidebar width/split/collapse) | Sidebar and UI state in config | PM-MUX / H-CFG |

## 10. Platform support

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Operating systems | macOS, Linux, Windows (x86_64; aarch64 Unix) | macOS, Linux, Windows generally available | PM-MUX / H-WIN |
| IPC transport | Unix domain socket vs Windows named pipe (with marker file) | Unix sockets vs Windows named pipes | PM-MUX / H-PLUG |
| PTY backend | portable-pty (ConPTY on Windows) | Vendored portable-pty 0.9 (ConPTY on Windows) | PM-MUX / H-README |
| Windows specifics | Daemon always foreground; `.cmd`/`.bat` resume via self-deleting bridge; liveness sweep inert | Capability table: persistent sessions, agent detection, notifications, sounds; plugins preview | PM-MUX / H-WIN |
| Installers | crates.io, per-platform release archives + SHA256SUMS | curl script, Homebrew, mise, Nix, `install.ps1` | PM-MUX / H-README |
| Self-update | No | `herdr update` (keeps compatible servers running), stable/preview channels | — / H-CLI |
| Mobile | Web/mobile client via the streaming server | Unsupported (Android/Termux explicitly no) | PM-MUX / H-CHANGELOG |

## 11. Security

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Socket ownership | Unix socket mode 0600; fallback dir 0700 verified before bind and connect; peer must run as daemon user | Owner-only by default; elevated Windows servers accept administrator clients only unless `allow_unelevated_clients` | PM-MUX / H-WIN |
| Named pipe | Owner-only SDDL; client-side server-identity check | Named pipes supported; specific ACL model not inventoried | PM-MUX / — |
| Pane-to-pane control | Full control socket in every pane **by design** (agents drive other panes); opt-in `--pane-endpoints` hook-only sockets (cap 256) | Panes receive the API socket; unelevated-client guard on Windows | PM-MUX / H-CLI |
| Input validation | Hook validation at the door: label charset, telemetry caps, stale `seq`, future timestamps rejected | Resume argv validation (PATH command, no apostrophes, ≤64 args, ≤8 KiB), control-char stripping, stale `--seq` rejection | PM-MUX / H-ADD |
| Env hygiene | Strips inherited `PAR_MUX_*` and outer agent identity markers (herdr parity) | Strips outer terminal and agent session markers so panes cannot impersonate the origin | PM-MUX / H-CLI |
| Remote credentials | n/a (no SSH tier) | Machine profiles store no passwords/keys; API payloads forwarded over SSH without shell interpolation | — / H-CLI |
| Untrusted output | Passthrough grants full host escape capability — prefer render (SECURITY.md) | Pane history off by default; session dirs should be treated like terminal history | PM-MUX / H-STATE |
| Repo trust | n/a | `--trust-repository` per-request Git trust for another user's repo | — / H-CLI |

## 12. Command surface

| Capability | par-mux | herdr | Refs |
| --- | --- | --- | --- |
| Style | tmux verbs sent over the socket via `--cmd`; subcommands for lifecycle | Noun-verb JSON CLI (`workspace/pane/agent/...`), identical arguments over the socket API | PM-MUX / H-CLI |
| Scale | ~47 wire commands, 8 subcommands, ~15 lifecycle/integration flags | ~15 noun groups with 3–20 verbs each (100+ commands) plus hidden bridge subcommands | PM-MUX / H-CLI |
| Full enumerations | See [Appendix](#appendix-command-enumerations) | See [Appendix](#appendix-command-enumerations) | — |

## Where each leads

**par-mux** is stronger on:

- tmux control-mode wire compatibility — tmux-fluent clients, scripts, and habits work unchanged.
- Streaming: every pane mirrors to web/mobile clients through `par-term-streamer`.
- Structured agent telemetry (model, context percentages, reset windows) surfaced in the roster.
- Screen + scrollback persistence across restarts by default.
- Opt-in per-pane hook-only endpoints as a containment tier.
- Rust embedding (`MuxClient` / `MuxServer` / `PaneFactory`) for programs that want a multiplexer as a library.
- Late-client replay of held pane state after reconnect.

**herdr** is stronger on:

- Agent breadth: 22 supported integrations + 5 self-reporting, with three-tier detection and `agent explain` diagnostics.
- Mouse-first UX: drag-everything, context menus, rich theming (11 themes, auto light/dark), sounds, onboarding.
- A plugin system with event hooks and link handlers.
- Git worktree workspaces as a first-class concept.
- Remote operation: SSH thin client, saved machines, remote scripting without a TUI.
- Orchestration primitives: `pane wait-output`, `agent prompt --wait`, `agent wait`, anti-stall gating.
- Self-update channels and polished installers; native conversation restore across a wider agent set.

## Appendix: command enumerations

### par-mux (binary flags and subcommands)

`par-mux [NAME]`, `--socket`, `--state-dir`, `--foreground`, `--no-attach`, `--stop`, `--restart`, `--cmd/-c`, `--list-servers`, `--pane-endpoints`, `--expose-control-socket`, `--gen-config [--force]`, `--install-skill [--skills-dir]`, `--install-agent-hooks`, `--uninstall-agent-hooks`, `--agent-hooks-status`, `--integration-status`, `--install-agent-extensions`, `--uninstall-agent-extensions`, `--heal-agent-hooks`; subcommands `start`, `stop`, `restart`, `status`, `ls`, `attach`, `config`.

### par-mux (wire commands)

`new-session`, `new-workspace`, `list-workspaces`, `select-workspace`, `switch-client`, `rename-workspace`, `kill-workspace`, `set-workspace-respawn`, `new-window`, `select-window`, `kill-window`, `rename-window`, `rename-session`, `kill-session`, `split-window`, `select-pane`, `pane-title`, `pane-info`, `pane-exited-replay`, `clear-history`, `resize-pane`, `swap-pane`, `break-pane`, `join-pane`, `move-window`, `swap-window`, `respawn-pane`, `kill-pane`, `list-panes`, `list-windows`, `list-sessions`, `has-session`, `list-agents`, `list-commands`, `client-snapshot`, `capture-pane`, `send-keys`, `refresh-client`, `set-buffer`, `set-client-colors`, `set-environment`, `show-buffer`, `paste-buffer`, `version`, `reload-config`, `show-options`, `set-option`, `kill-server`.

### herdr (CLI noun groups)

- Global: `--session`, `--remote`, `--machine`, `status`, `update [--handoff]`, `channel`, `completion`, `--default-config`, `--skill`
- `api` (schema, snapshot), `server` (stop, reload-config, agent-manifests, update-agent-manifests, reload-agent-manifests)
- `machine` (list, add, rename, disable, enable, remove, status, reconnect), `session` (list, attach, stop, delete)
- `notification show`, `workspace` (list, create, get, focus, rename, report-metadata, close), `worktree` (list, create, open, remove)
- `tab` (list, create, get, focus, rename, close)
- `pane` (list, current, get, layout, process-info, neighbor, edges, focus, resize, zoom, rename, input, split, swap, move, close, read, send-text, send-keys, run, wait-output, report-agent, report-agent-session, release-agent, report-metadata)
- `agent` (list, get, read, send-keys, prompt, rename, focus, wait, attach, start, explain)
- `terminal` (attach, session control, session observe, title set/clear)
- `integration` (install, uninstall, status), `plugin` (install, update, list, uninstall, enable, disable, link, unlink, config-dir, action list, action invoke, log list, pane open/focus/close), `config reset-keys`

## Sources

Verified against these trees on 2026-10-19 (read-only exploration; citations are section/file level):

| Key | Source |
| --- | --- |
| PM-MUX | `docs/MUX.md` (par-mux operational reference, sections cited per row) |
| PM-CLI | `crates/par-mux/src/bin/par_mux/main.rs` (clap surface) |
| PM-SRC | `crates/par-mux/src/mux/` — `layout.rs`, `persist.rs`, `hooks/`, `server/`, `command/`, `attach/` |
| H-README | herdr `README.md` |
| H-CONCEPTS | herdr `docs/next/website/src/content/docs/concepts.mdx` |
| H-CLI | herdr `docs/next/website/src/content/docs/cli-reference.mdx` |
| H-KEYS | herdr `docs/next/website/src/content/docs/keyboard.mdx` |
| H-AGENTS | herdr `docs/next/website/src/content/docs/agents.mdx` |
| H-STATE | herdr `docs/next/website/src/content/docs/session-state.mdx` |
| H-API | herdr `docs/next/website/src/content/docs/socket-api.mdx` |
| H-PLUG | herdr `docs/next/website/src/content/docs/plugins.mdx` |
| H-ADD | herdr `docs/next/website/src/content/docs/add-herdr-support.mdx` |
| H-WIN | herdr `docs/next/website/src/content/docs/windows-beta.mdx` |
| H-CFG | herdr `src/main.rs` (`DEFAULT_CONFIG`) |
| H-SKILL | herdr `skills/herdr/SKILL.md` |
| H-AGENTS.md | herdr `AGENTS.md`, `distribution/agent-guide.md` |
| H-CHANGELOG | herdr `docs/next/CHANGELOG.md` |

herdr's released documentation snapshots live under `docs/versions/0.9.5/`; the `docs/next/` tree is the current source of truth.
