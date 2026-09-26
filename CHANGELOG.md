# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security
- **Kitty graphics file media is gated (`t=f`/`t=t` no longer reads or deletes arbitrary paths)** (`src/graphics/kitty.rs`). Terminal output is untrusted input, but a single `t=t` APC naming an absolute path deleted that file after reading it — before even checking it was an image (audit SEC-101, CWE-73/CWE-22; also closes the SEC-102 `t=f` read and the SEC-103 symlink race). The new `FileMediaMode` gate defaults to `temp_only`: `t=t` loads only a file whose canonicalized path sits under an allowed temp root (`$TMPDIR`, `/tmp`, `/dev/shm`) **and** whose filename contains `tty-graphics-protocol` (kitty's own spec rule), opens it with `O_NOFOLLOW`, and deletes it only after the payload decodes as an image. `t=f` now requires opting in. **Behavior-affecting/breaking:** tools relying on unrestricted `t=f`/`t=t` must opt in via `Terminal::set_allow_file_media` / `Terminal.set_allow_file_media("all")` in Python, or the new `kitty_file_media` field on `StreamingConfig` (`"off"` / `"temp_only"` / `"all"`, default `"temp_only"`) which the streaming server applies to every session terminal it creates. Sixel and iTerm2 media are unaffected (they take no paths).

### Added
- `Terminal.set_allow_file_media(mode)` / `Terminal.get_allow_file_media()` — Python bindings for the Kitty file-media gate, with `StreamingConfig.kitty_file_media` for streaming session terminals.

## [0.52.0] - 2026-09-25

### Security
- **par-mux: the Linux socket fallback no longer lives in shared `/tmp` unguarded** (`src/mux/ipc.rs`). Without `$XDG_RUNTIME_DIR`, `default_socket_path` returned `/tmp/par-mux-<name>.sock` directly, and `connect_or_spawn` attached to whatever server was listening there — another local user could pre-bind the path and receive a client's keystrokes, `set-environment` values, and clipboard. The fallback base is now the tmux-style per-UID directory `<tmp>/par-mux-<uid>/` (mode `0700`), and both `prepare_socket_path` (bind) and `connect_local_stream` (connect) verify the directory's owner and mode before using it, failing closed with `PermissionDenied` when it is owned by another user, grants group/other access, or is not a directory. Accepted connections are additionally refused unless the peer runs as the daemon's user (`SO_PEERCRED`/`getpeereid`), dropped without disturbing the listener. Explicit `--socket` paths and `$XDG_RUNTIME_DIR` bases are unchanged. **Behavior-affecting:** the default fallback socket path moved one directory deeper, so daemons running from before the upgrade are not found at the new path (a fresh daemon spawns; kill the old one); persisted state is keyed by socket stem and is unaffected. See [docs/MUX.md](docs/MUX.md#socket-and-state-paths).

### Added
- **par-mux panes export their session and window identity** (`src/mux/pane.rs`, `src/mux/tree.rs`, `src/mux/persist.rs`, `src/bin/par_mux/main.rs`). Every pane now also carries `PAR_MUX_SESSION_ID` (`$N`), `PAR_MUX_SESSION` (name), `PAR_MUX_WINDOW_ID` (`@N`), and `PAR_MUX_BIN` (the daemon executable, so a pane script can run `"$PAR_MUX_BIN" --socket "$PAR_MUX_SOCKET" --cmd …` without `par-mux` on `PATH`), set on `new-session`, `new-window`, `split-window`, and restore. The values are fixed at spawn, as tmux's are. See [docs/MUX.md](docs/MUX.md#agent-hook-reports).

- **par-mux per-session environment: `set-environment` and `new-session -e`** (`src/mux/command.rs`, `src/mux/tree.rs`, `src/mux/dispatch.rs`, `src/mux/persist.rs`). `set-environment -t $N NAME VALUE` / `-u NAME` and `new-session -e NAME=VALUE` (repeatable) maintain a per-session environment applied on top of the daemon's for every pane spawned afterwards; running panes are untouched, as in tmux. A client that reconnects with a fresh `SSH_AUTH_SOCK`, `DISPLAY`, or `PATH` sends it here, since the daemon outlives its clients and would otherwise hand new panes its own stale startup environment. The map persists with the session; state files written before it existed load with an empty one. See [docs/MUX.md](docs/MUX.md#command-reference).

- **par-mux `split-window -c` / `new-window -c`** start the new pane in a given directory.
- **par-mux `-t` targets resolve by name**: pane user title, window name, and session name.
- **par-mux `set-buffer` quoting and `-H` hex payload.**
- **par-mux client metrics**: `set-client-colors` reports the client theme for OSC 10/11 answers; `refresh-client -p` reports the client's cell pixel size daemon-wide.
- **par-mux `%sessions-changed`** on session create/destroy, and `exit-when-empty` with a deliberate final save.
- **par-mux `%agent-released` notification** and a liveness sweep that clears hook claims whose agent provably died.
- **`PtySession::fire_output_callback`** for daemon-fed output.

### Changed
- **par-mux refuses to start a nested daemon, and panes drop a stale outer `PAR_MUX_*` env** (`src/bin/par_mux/main.rs`, `src/mux/client.rs`, `src/mux/mod.rs`, `src/pty_session.rs`). Serve mode run with `PAR_MUX_ENV=1` (inside a mux pane) exits non-zero naming the `PAR_MUX_ALLOW_NESTED=1` override and binds no socket; `MuxClient::connect_or_spawn` attaches to a running daemon from inside a pane but refuses to auto-spawn one (fail-fast instead of a ten-second retry on a socket the guard keeps unbound). `--cmd`, `--stop`, and `--restart` are exempt, as tmux exempts `kill-server`. Separately, any PTY spawned by a process inside a mux pane now drops every inherited `PAR_MUX_*` variable (prefix-matched) and re-adds only its own via `set_env` — a par-term started in a pane no longer reports its local tabs' agents to the outer daemon under the wrong pane id. See [docs/MUX.md](docs/MUX.md#nested-daemons).
- **par-mux state file is owner-only from creation** (`src/mux/persist.rs`). The temp file was created with the process umask and chmodded to `0600` after the write; it is now opened `0600`, so no readable window exists. This matters now that the file can hold session environment values.
- **`par-mux --cmd` no longer echoes the full command on a transport failure**, only its name, since arguments can carry environment values.
- **Breaking for embedders: `PaneFactory::create_pane` takes a `&SpawnContext`** (`src/mux/pane.rs`). The context carries the owning session and window and the session environment. `ShellPaneFactory` and `AgentPaneFactory` gain a `bin_path` field exported as `PAR_MUX_BIN`. Custom factories add the parameter and pass it through to `ShellPaneFactory::create_pane` when they delegate.

### Fixed
- **par-mux pane lifecycle fixes** (`src/mux/`): killed panes are reaped and the kill leaves the tree lock free; `kill-pane` emptying a window broadcasts `%window-close`; a failed agent resume no longer deletes the restored pane; each pane's cwd persists and restore returns panes to it; a raced shutdown keeps the last-good snapshot; `pane.release_agent` clears a dead agent's claim; report freshness is tracked per source; reported agent state and label are validated at the hook door.
- **par-mux protocol fixes**: a non-UTF-8 client line is a `%error` reply, not a disconnect; the client matches `%end`/`%error` to its `%begin` command number; a listener fault no longer waits on silent clients to exit.
- **par-mux herdr parity**: panes drop outer agent identity env; scrape `contains` matching folds case; scrape regex/line_regex lists require all patterns.
- **Kitty `t=t` images render in mux panes** — daemon terminals retain their temp files.
- **Windows: unix-only shell binding is cfg-gated**, removing a build warning.
- **`make clean` no longer deletes tracked files** or gate-critical artifacts.
- **PTY output callbacks now fire after the bytes are applied to the terminal** (`src/pty_session.rs`, `src/mux/pane.rs`). The reader thread invoked the output callback before `process_deferred` applied the read to the terminal, so a consumer that both forwards raw bytes and reads terminal state could observe bytes the state did not yet contain — the root cause of the `mux_reattach` seed flake, where a reattached client's replay seed could legitimately miss output (e.g. the cursor-hide escape) its new `%output` subscription then delivered, leaving a TUI replaying onto the main screen. The callback now runs after the write guard drops and device replies are written, still receiving the raw unmodified bytes. **Behavior-affecting for embedders** (documented contract change, Python-visible through the streaming/mux paths that consume the callback): code that relied on the old "callback before processing" ordering — none in this crate — must read state after the callback instead of around it.
- **par-mux raises its `RLIMIT_NOFILE` soft limit at daemon start** (`src/mux/server.rs`). The daemon inherited its spawner's descriptor limit: under launchd or the Dock on macOS that is a soft limit of 256, and at roughly four descriptors per pane the 60th pane was where `new-window` started failing with EMFILE (herdr raises its server to 8192 the same way). Binding a server now raises the soft limit toward the hard limit, logging old and new values; an unbounded hard limit (the macOS default) is treated as 8192. A test holds the process at a 256-descriptor soft limit and opens 70 sessions. See [docs/MUX.md](docs/MUX.md#building-and-running).
- **Windows: par-mux agent resumes spawn with exact arguments instead of failing** (`src/mux/win_resume.rs`, `src/mux/pane.rs`, `src/mux/persist.rs`). Every Windows agent-resume failed: the restore path POSIX-single-quoted the argv and ran it through `cmd.exe /C`, which treats `'` literally — a resumed agent ran a program literally named `'claude'` — while `& | ^ %` in a session path stayed live (injection bounded by the socket's same-user trust). The restore now hands structured argv to the factory through a new `PaneFactory::create_argv_pane` seam (default: the previous string path, so custom factories and POSIX behave unchanged). On Windows, an `argv[0]` resolving to a PE image spawns directly — spaces, `&`, and quotes arrive verbatim, no shell — while `.cmd`/`.bat` shims (the npm-distributed agent CLI shape) and unresolved names run through a self-deleting bridge batch under `%COMSPEC% /d /c`. `%VAR%` expansion and embedded double quotes remain unrepresentable on a cmd line. Verified on the Windows 11 VM: a restored pane whose session path carries a space, an `&`, and a quote receives the exact arguments (`a_windows_resume_receives_the_exact_arguments`). See [docs/MUX.md](docs/MUX.md#persistence-and-restart).
- **par-mux no longer dies with the terminal that started it** (`src/mux/client.rs`, `src/bin/par_mux/main.rs`). The auto-spawned daemon ran in the spawner's process group with a controlling tty, so a terminal hangup or Ctrl-C killed it — and every pane — with no final save; `par-mux --restart NAME &` stayed in the shell's job with the same failure. The auto-spawned daemon now `setsid`s into its own session before exec, `--restart` detaches before serving (fork + `setsid` + stdio to `/dev/null`, returning immediately — no `&` needed), and the serving daemon ignores SIGHUP, SIGINT, SIGQUIT, SIGTSTP, and SIGPIPE (tmux's server ignore set; SIGTERM remains the clean shutdown path). `MuxClient::spawned_daemon_pid` exposes the auto-spawned daemon's pid for session/process-group assertions. See [docs/MUX.md](docs/MUX.md#shutdown-semantics).
- **par-mux accept faults no longer kill the daemon without a save** (`src/mux/server.rs`). EMFILE on the accept loop broke it silently: the daemon exited with nothing logged, the state file unwritten, and a stale socket left behind (measured: `ulimit -n 48` plus 40 raw connections). EMFILE, ENFILE, and ECONNABORTED now back off for a second and keep serving (tmux's server pauses on ENFILE/EMFILE the same way), and any other listener fault is logged and takes the same final-save path as SIGTERM. See [docs/MUX.md](docs/MUX.md#shutdown-semantics).

## [0.51.0] - 2026-09-24

### Added
- **`par-mux --cmd` client mode** (`src/bin/par_mux/main.rs`, `src/mux/client.rs`). `par-mux [<name>|--socket <path>] --cmd '<command>'` (short `-c`) sends one control command to the running daemon, prints the reply body one line per stdout line with the `%begin`/`%end` framing stripped, and exits 0; a `%error` reply prints the daemon's message to stderr and exits 1, as does a socket no daemon owns (client mode never spawns a daemon); a transport failure exits 2. Scripts and agents can now drive panes (`par-mux --cmd 'send-keys -t %3 "ls -la" Enter'`, `capture-pane`, `split-window`, `list-agents`) without raw socket I/O or linking the library. See [docs/MUX.md](docs/MUX.md#client-mode).
- **`MuxClient::send_checked` and `mux::Reply`** (`src/mux/client.rs`). `send` returned the same `Ok(body)` for a `%end` and a `%error` block, so a caller could not tell a failed command from a successful one; `send_checked` returns `Reply { body, ok }`. `send` keeps its existing contract.
- **Mux-backed streaming sessions: `MuxSessionFactory` and `par-term-streamer --mux-socket`** (`src/streaming/mux_factory.rs`, `src/bin/streaming_server/`; features `streaming` + `mux`). A streaming session can now mirror a pane owned by the par-mux daemon instead of spawning its own shell, so the existing web and mobile frontend can view and drive mux panes (the herddeck enabler). The mirror is seeded from the daemon's screen replay and follows the pane's `%output`; input and mouse reach the pane as `send-keys -H`; the size policy is latest-resize-wins, with the mirror starting at the pane's size so opening a viewer never resizes the pane. `?session=pane-N` selects the pane. See [docs/STREAMING.md](docs/STREAMING.md#mux-backed-sessions).
- **`pane-info -t %N` mux command** (`src/mux/dispatch.rs`). Replies `%N @W COLSxROWS`: the pane's window and current grid size, read-only. A mirroring client seeds at the pane's own size with it instead of resizing the pane to its viewport.

- **Web frontend: size policy for viewers of a shared pane** (`web-terminal-frontend/components/Terminal.tsx`, `lib/viewport.ts`, `components/OnscreenKeyboard.tsx`, `app/page.tsx`). A viewer larger than the pane dot-fills the surplus (tmux style); a phone shows the full grid fit to the width with pinch-zoom and drag-pan, and sends a resize only on a deliberate action (the new Fit button on the keyboard bar, or opening the on-screen keyboard). No viewer echoes a server-sent resize back any more. See [docs/STREAMING.md](docs/STREAMING.md#mux-backed-sessions).

### Changed
- **par-mux daemon resolution falls back to PATH** (`src/mux/client.rs`). `MuxClient::connect_or_spawn_at` now tries the `par-mux` binary next to the current executable first, then every `PATH` entry that holds `par-mux`. A candidate that cannot be started advances the search; any other spawn error surfaces with its path, and exhausting the list names every path tried. Embedders such as par-term no longer need to ship `par-mux` beside their own binary.
- **Behavior-affecting: phones no longer resize a plain streaming session's PTY on connect** (`web-terminal-frontend/components/Terminal.tsx`). A viewer detected as a phone (width under 640 px or a mobile user agent) now opens at the server's current size, fit to its width, and resizes the PTY only on the Fit button or when opening the on-screen keyboard. Desktop viewers are unchanged.

### Fixed
- **macOS: a PTY child that exits before its output is read no longer loses that output** (`src/pty_session.rs`). On macOS, once the last slave descriptor closes, the pty discards output the master has not read yet. `PtySession` dropped its slave right after spawn, so a fast child (such as `echo`) that exited before the reader thread's first `read()` lost its whole output: the grid stayed empty and the update generation never advanced. This affected Python `PtyTerminal`, par-mux panes, and streaming sessions, and showed up as the intermittent `test_generation_counter_increments_on_pty_output` failure under load. `PtySession` now keeps its slave open on macOS for the session's lifetime. The master still reads EOF when the child exits, because the session leader's exit revokes its controlling tty. Linux keeps unread output readable after the slave closes, so it is unchanged.

## [0.50.0] - 2026-09-23

### Removed
- **The dead terminal multiplexing module is removed — breaking Python API change** (`src/terminal/multiplexing.rs`, `src/python_bindings/types/session.rs`, `src/python_bindings/terminal/multiplexing_api.rs`). `PaneState`, `WindowLayout`, `SessionState`, and the `Terminal` pane-state methods (`capture_pane_state`, `restore_pane_state`, `set_pane_state`, `get_pane_state`, `clear_pane_state`, `create_window_layout`, `create_session_state`, `serialize_session`, `deserialize_session`) are gone from both the Rust API and the Python bindings (`_native.pyi` regenerated). The module had zero callers, its capture was lossy by type (`Vec<String>` content — no colors, attributes, scrollback, or alternate screen), and `restore_pane_state` restored only size/title/cursor by its own comment. The removal was decided against adoption because `replay_snapshot.rs`'s `TerminalSnapshot` already captures what a real persistence format needs (both grids, cursors, colors, modes, scrollback); par-mux Phase 3's on-disk format starts from that type instead. Migration: nothing to migrate to — no consumer of these types existed in this codebase; if external code used them, keep the capture/restore semantics you need via `TerminalSnapshot`-shaped data.
- **Six dead public functions** (QA-112; `src/debug.rs`, `src/grapheme.rs`, `src/macros.rs`). `log_vt_input`, `log_cursor_move`, `log_grid_op`, and `log_mode_change` (superseded by the `debug_*!` macros routing through `logf`), `is_wide_grapheme_with_config`, and `MacroPlayback::reset` are deleted — each re-verified dead (zero callers) against this repo and both sister repos (`par-term`, `par-term-emu-tui-rust`) before removal. Breaking only for external code that called them; the crate's own caller count was zero.
- **`Renderer::contains_regional_indicators`** (QA-112; `src/screenshot/renderer.rs`). The string-based Regional Indicator probe (kept only for tests under `#[allow(dead_code)]`) is deleted along with the six tests that exercised it; `render_grid` uses the cell-scanning `row_has_regional_indicators` path, which `docs/REGIONAL_FLAG_LIMITATION.md` now quotes instead.

### Security
- **Read-only streaming clients can no longer mutate shared state via Resize, FocusChange, Selection, or Clipboard messages (SEC-001)** (`src/streaming/server.rs`). The `read_only` guard existed only on Input/Mouse/Paste; a `?readonly=1` viewer could still resize the shared PTY, inject OSC focus sequences, mutate the selection broadcast to every client, or set/get the session clipboard. The same first-statement guard now covers all four message arms on both the tungstenite and axum paths; clipboard "get" is additionally gated on `allow_clipboard_read` (the existing OSC 52 read flag).
- **Selection coordinates are bounds-checked before byte slicing (SEC-002)** (`src/terminal/screen.rs`, `src/streaming/server.rs`). `get_selected_text` used display columns as UTF-8 byte offsets into the exported text, so a client-supplied start column landing mid-grapheme panicked the connection task. `set_selection` now clamps coordinates to the grid (end column may equal `cols` — word selections report an exclusive end) and maps them to char-boundary byte offsets; the streaming `SelectionRequest` handler additionally rejects out-of-range coordinates outright.
- **WebSocket handshakes are timeout-bounded and pre-handshake connections count against `max_clients` (SEC-004)** (`src/streaming/server.rs`). Both tungstenite listeners reserve a global client slot before awaiting the handshake (holding the RAII guard across it) instead of after, closing the accept-to-slot TOCTOU window; the handshake (including TLS) is wrapped in a 10 s timeout that drops connections which never complete it. Previously an unauthenticated pre-upgrade connection could occupy a task indefinitely, uncapped by `max_clients`.
- **TLS private-key permission checks now also run for `--tls-pem` (SEC-006)** (`src/streaming/config.rs`). The 0600/0400 world-readable rejection that `from_files` (separate cert/key files) already applied is extracted into a shared helper and now runs in `from_pem` too, whose combined PEM contains the private key; `--tls-pem` previously accepted world-readable key material silently.
- **HTTP Basic auth verification runs at constant work regardless of username (SEC-008)** (`src/streaming/config.rs`). `HttpBasicAuthConfig::verify` no longer returns early on a username mismatch — both the username comparison and the password check (bcrypt against a fixed dummy hash when the username doesn't exist) always run and are ANDed at the end, closing a username-enumeration timing side channel.
- **Secrets no longer appear in `Debug` output (SEC-009)** (`src/streaming/config.rs`). `PasswordConfig` and `StreamingConfig` no longer derive `Debug`; hand-written impls redact the password/API key, so `{:?}` formatting can no longer leak credentials into logs or error messages.
- **`X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'` are sent on every HTTP response (SEC-010)** (`src/streaming/server.rs`). The served web frontend drives a live shell; without these headers a transparent iframe on an attacker page plus cached Basic credentials lets the attacker clickjack keystrokes into the terminal. Applied at the outermost router layer, so both static files and API routes are covered.
- **The global client slot is reserved before session creation, and session ids are validated (SEC-011)** (`src/streaming/server.rs`). The axum WebSocket prologue now reserves its slot before resolving or creating a session, so a server at `max_clients` rejects a new connection without spawning a PTY for a connection that will be refused anyway. Session ids from the query string must match `[A-Za-z0-9_-]{1,64}`; a malformed id (path-traversal shapes, oversized values) falls back to the default session instead of reaching the registry.
- **Frontend bundle download: hard size cap and guarded web-root replacement — new `--force-web-download` flag** (`src/bin/streaming_server/frontend_download.rs`, `src/bin/streaming_server/cli.rs`; SEC-007). `--download-frontend` now streams the archive with a 50 MiB cap (rejecting oversized bodies mid-stream instead of buffering whatever the URL serves) and refuses to clear an existing web root that has no `index.html` — a directory that does not look like a previously extracted frontend bundle — unless the new `--force-web-download` flag is passed. Release-side `.sha256` sidecar verification is not wired yet: no release currently publishes checksum assets; wiring it is follow-up work for the release workflow.
- **PTY writes are off the async runtime; Input/Paste payloads are capped — behavior-affecting** (`src/streaming/server.rs`; SEC-005). `Input` messages larger than 64 KiB and `Paste` messages larger than 256 KiB are now logged and dropped, and all Input/Paste PTY writes run on `tokio`'s blocking pool with no writer or terminal lock held across the write. Previously a non-reading foreground process (full kernel PTY buffer) froze the session for every client: the blocking `write_all` stalled the tokio worker, serializing all other message handling behind it.
- **OSC/DCS parser accumulation is now capped incrementally — behavior-affecting default change** (`src/terminal/mod.rs`, `src/terminal/sequences/osc/mod.rs`, `src/terminal/sequences/dcs/mod.rs`; SEC-003). `DEFAULT_MAX_OSC_DATA_LENGTH` drops from 128 MiB to 1 MiB: the old value only bounded the *dispatch*-time check, while vte 0.15's internal OSC buffer (which has no per-byte hook) grew unbounded until that check fired. The cap is now enforced in `Terminal::advance_parser` — payload bytes past `max_osc_data_length` are never fed to vte, terminators always are (so the parser never desyncs), and the truncated dispatch is dropped whole. The non-Sixel DCS accumulation buffer is capped at 64 KiB with the overflowed sequence dropped at unhook. Deployments pushing larger inline images (iTerm2/Kitty base64 in OSC 1337) can raise the limit with `set_max_osc_data_length`.

### Added
- **par-mux `kill-server` command and `par-mux --stop` / `--restart`** (`src/mux/command.rs`, `src/mux/dispatch.rs`, `src/bin/par_mux/main.rs`, `docs/MUX.md`). There was previously no way to stop or restart the daemon short of finding its PID and sending SIGTERM. `kill-server` (tmux's name) raises the same shutdown flag SIGTERM does — the reply block goes out, the accept loop exits, the final state save runs, and `%exit` reaches every client — and is refused with an error for an embedded server with no shutdown handle. `par-mux [NAME | --socket PATH] --stop` sends `kill-server` and waits (30 s bound) until the socket stops accepting; "no daemon running" is not an error. `--restart` stops, then serves the same socket in this process, restoring the tree the stop just saved — the routine fix for a daemon that outlives a rebuild: clients attach to whatever daemon owns the socket, so the old one keeps serving old code until restarted.
- **par-mux `version` command reporting the daemon's build stamp** (`build.rs` new, `src/mux/command.rs`, `src/mux/dispatch.rs`, `src/mux/mod.rs`, `docs/MUX.md`). The daemon outlives its clients, so a stale daemon silently serves arbitrarily newer clients — every daemon-side fix reads as "didn't work" until someone notices the daemon predates the build. `build.rs` bakes `PAR_TERM_CORE_BUILD_SHA` (short git sha, `-dirty` marked, `unknown` outside a repository) into the crate; `mux::build_stamp()` exposes `<crate version>+<git sha[-dirty]>` (`+unknown` when built outside a repository) to both sides of a daemon/client pair, and the read-only `version` command answers with it — tree-free by design so a long-lived daemon can always answer it.
- **par-mux reaps panes whose child exited — tmux's pane-dies-with-process semantics** (`src/mux/server.rs`; `tests/mux_daemon.rs`). Nothing previously noticed a pane's child exiting: the PTY reader flips `is_running` on EOF, but the pane sat dead in the tree forever and clients stared at a frozen pane (typing `exit` in a mux pane exited the shell and stuck exactly there). The accept loop's idle tick now runs a reaper every 250 ms: a dead pane is removed from its window with `%layout-change` + `%window-pane-changed` broadcasts, or its window is closed (`%window-close`) when it held the last pane — the notifications tmux consumers already handle. An emptied session is left in place for create-or-attach to refill.
- **`PtySession::mark_updated()` — advance the update generation from outside the reader thread (Rust)** (`src/pty_session.rs`). A childless session (a par-mux pane fed via the host's own PTY reader rather than its own child process) never advanced `update_generation`, since only the reader thread's read loop bumped it — a generation-keyed render cache would serve stale cells forever. `mark_updated()` performs the same generation bump plus condvar wake for such external feeds. Rust-only; no Python binding exists for it.
- **par-mux pane titles: `select-pane -T`, `%pane-title-changed`, `pane-title` query, persisted across restart (Rust `mux` feature)** (`src/mux/command.rs`, `src/mux/dispatch.rs`, `src/mux/pane.rs`, `src/mux/persist.rs`, `src/tmux_control.rs`, `src/mux/emit.rs`, `src/python_bindings/types/notification.rs`, `docs/MUX.md`; card `01a0cea631f27a6086bd45552987a16d`). `select-pane -t %N -T 'title'` sets a user pane title (quoted grammar, spaces allowed; `-T ''` clears), every connected client receives `%pane-title-changed %N <title>` on set and on clear (title = rest of line; no title token = cleared), and a new read-only `pane-title -t %N` replies with the pane's effective title for a client attaching after a change. Precedence is a documented divergence from tmux: the user title is sticky — the pane program's OSC 0/2 title never overwrites it — while with no user title the pane reports the program's live OSC title. The title persists in the save file (`user_title`, serde-defaulted so older files load unchanged) and a daemon restart restores it. The Python `TmuxNotification` surface gains the `pane-title-changed` arm (`pane_id` + `name` = new title). par-term's UI for setting/rendering a title is deliberately out of scope here.
- **Application keypad mode is tracked, persisted, and restored (Rust + Python API)** (`src/terminal/mod.rs`, `src/terminal/sequences/esc.rs`, `src/terminal/replay_snapshot.rs`, `src/terminal/semantic_snapshot.rs`, `src/streaming/session.rs`; card `01a0cd0a5ef87e03b91adb34da0a10dc`). `ESC =` / `ESC >` (DECPAM/DECPNM) were silently ignored, so keypad mode could not be queried, snapshotted, or restored — a full-screen app that sets it (vim, less) lost the mode on mux reattach or replay restore. `modes.application_keypad` now tracks the two ESC dispatch arms (a `ModeChanged` event on the flip, none on a no-op re-send; RIS resets via the modes default), `TerminalSnapshot` persists the flag, `export_screen_restore_sequence` emits `ESC =` when the mode is set, the streaming `Connected` message reports it alongside DECCKM, and `Terminal.application_keypad()` exposes it to Python. Flag-only — key encoding is unchanged.
- **Coverage-guided fuzz targets for the four untrusted-byte parsers (Rust)** (`fuzz/`, `Makefile`, `.github/workflows/fuzz.yml`, `CONTRIBUTING.md`; ENH-014). A cargo-fuzz crate (outside the workspace, `rust-only` features so no Python toolchain) with libFuzzer targets for `Terminal::process` (including the export walkers on the mutated state), the Sixel state machine (asserting `SixelLimits` holds against mutated input), the Kitty graphics APC parser (mirroring the real caller's m=1-continuation vs final-chunk semantics), and the tmux control-mode parser (whole-input plus every split-point parse for partial-line buffering). Committed seed corpora (`fuzz/corpus/*/*.seed`), `make fuzz-all` / `make fuzz-<target>` (default 60 s each, not part of `checkall`), a nightly + manual-dispatch CI workflow (10 min per target, crash artifacts uploaded on failure), and a documented crash-to-regression-test policy in CONTRIBUTING.md. No library code changed.
- **par-mux `capture-pane -e` — styled capture, and a full-state reattach seed (Rust `mux` feature)** (`src/mux/command.rs`, `src/mux/dispatch.rs`, `src/grid/export.rs`, `docs/MUX.md`; card `01a0cbfaeb2c7111910e9b302d934cff`). `capture-pane` gains tmux's `-e`: the visible screen (the active grid, so an alt-screen TUI captures its TUI screen) as one line per grid row with SGR escape bytes inline — a styled run precedes its text, a row that used SGR ends with a reset before its newline, unstyled rows stay plain text, and empty rows are empty lines (always exactly one line per grid row; the framing is line-per-row, not cursor addressing). Without `-e` the reply stays the plain logical-lines capture, byte-identical to the pre-`-e` reply; `-e` composes with `-S/-E` over the styled scrollback+screen composition. `refresh-client -t` (the reattach seed) now replies with `Terminal::export_screen_restore_sequence()` instead of the styled screen alone, so a reattached client additionally restores the cursor position/visibility/style and the input modes (DECCKM, bracketed paste, focus, mouse, origin) that a full-screen app's subsequent `%output` deltas assume — the reply's wire shape is documented in `docs/MUX.md`. A live-daemon test drives a full-screen-TUI-shaped pane (alt screen, styled tags, hidden cursor) through detach → reattach → seed → subsequent `%output` and asserts the client emulator's grid matches the pane's, attributes included — a plain-`content()` seed fails it. (Superseded by the post-release scrollback fix below — see "par-mux reattach seed now replays scrollback" under Fixed.)
- **VT screen-restore encoder: `Terminal::export_screen_restore_sequence()` (Rust)** (`src/terminal/semantic_snapshot.rs`, `src/grid/export.rs`). Encodes the current state as a VT byte stream that, replayed into a fresh `Terminal` of the same size, reproduces the visible screen cell-for-cell including attributes (SGR-diffed via the existing styled-export walker) plus the state a full-screen app depends on: alt-screen (enter 1049 when active), scroll region, cursor position/visibility/style (DECSCUSR), and the input modes — DECCKM, bracketed paste, focus tracking, mouse tracking mode and encoding, origin mode (restored last with a region-relative re-position, since enabling it homes the cursor). Keypad mode and pending-wrap state are not modeled as replayable sequences and are not encoded. A round-trip test drives a full-screen-TUI-shaped stream (see the application-keypad entry above for the keypad half) (alt screen, 256/truecolor SGR, attribute mix, wide chars, combining char, styled trailing blanks) and asserts grid equality plus every restored mode. Groundwork for the par-mux reattach follow-on (capture-pane `-e`, wire-shape docs).
- **`PtyTerminal.wait_for_update()` / `wait_for_text()` blocking waits (Python API + Rust)** (`src/pty_session.rs`, `src/python_bindings/pty.rs`, `tests/test_pty.py`; ENH-011). A condition variable signalled by the reader thread the moment applied content is visible (the post-write-guard generation bump) — sub-millisecond wakeups where `time.sleep` polling would miss or wait long, and no race between spawn and first assert. `wait_for_update(since, timeout)` blocks until the generation advances past `since` (returns the new generation, or `None` on timeout or child exit); `wait_for_text(needle, timeout, scrollback)` re-checks after every applied update until the text appears. Both release the GIL while blocking — verified by a liveness test (a concurrent Python thread keeps running during a blocked wait), since `PtyTerminal` does not expose observers to deadlock against. Rust callers get `PtySession::wait_for_update`/`wait_until` plus a `Clone`-able `UpdateWaiter` handle (`PtySession` itself is not `Sync`; the waiter carries only the `Arc`s the wait needs).
- **par-mux Phase 6 Task 6.4: the restart proof — resumed, not merely same-id (Rust `mux` feature)** (`tests/mux_agents.rs`; `tests/assets/par-mux-fake-agent.sh` new). A live-daemon wire test drives the full Phase 6 arc end to end: a fake agent — a pane PROCESS, not a herdr port — reports its own session identity with a resume invocation pointing back at itself, a clean SIGTERM saves, and the restarted daemon must respawn the pane through that invocation. The assertion implements the design's restated criterion with both halves non-circular: the screen shows the resume marker carrying the SAME session id the daemon built into the argv from the persisted identity, and the resumed process's own report (same id, `session_start_source=resume`) lands in the second save — proven by its mode-tagged source (`par-mux:fx:resume`), which the restored startup-tagged identity cannot produce. Red-proofed against a fresh-spawn build (resume chain disabled, rebuilt, run): the test fails at the marker assertion with `left: ""` vs `right: "FAKE-AGENT resume fx-42"`. The script re-announces its marker for a minute because restore re-hangs the saved screen after spawn, so a single early print can race the snapshot. Phase 6 is complete: identity persists (6.1), the table builds invocations (6.2), restore uses them with structural fall-back (6.3), and a restart is now proven to resume.
- **par-mux Phase 6 Task 6.3: restore wires the resume invocation with structural fall-back (Rust `mux` feature)** (`src/mux/persist.rs`; `src/mux/agent_resume.rs`). `from_persist_state` now computes an effective spawn command per pane — the persisted agent session's resume invocation (hook-reported argv verbatim, else the 6.2 per-agent table) rendered for the factory's `sh -c` spawn by the new `render_argv`, which single-quotes every argument unconditionally: the argv crosses from a validated structure into a string a shell re-parses, and picking an "unquotable" alphabet is a guess about vendor CLIs the table exists not to make. The fail-safe is structural, not a rescue path — every failure mode of the chain is an `Option` degrading to the pane's original `spawn_command` (exactly Phase 3 behavior), so a missing table entry, a ref the table cannot use (a path for an id-only agent), or an uninstalled agent binary all land on "spawn the pane as it was"; the binary case is contained inside the pane by the shell's own command-not-found, with deliberately NO retry, probe, or timeout (an agent that accepts a resume flag and silently starts fresh is task 6.4's after-the-fact question, not spawn time's). Non-agent panes restore through the unchanged Phase 3 path, proven by `assert_same_shape` plus a re-capture carrying no agent session. Unit tests restore through a `RecordingFactory` that records the computed command and delegates the actual spawn to a bounded sleeper, so restoring a pi/omp/claude pane never launches a real agent CLI from a test.
- **par-mux Phase 5 agent layer (Rust `mux` feature)** (`src/mux/hooks.rs` new, `src/mux/pane.rs`, `src/mux/command.rs`, `src/mux/server.rs`, `src/bin/par_mux/main.rs`; `tests/mux_hooks.rs`, `tests/mux_agents.rs`, `tests/assets/par-mux-agent-state.sh`). herdr's agent-hook integrations port with an env-var rename: a control-socket line whose first non-whitespace byte is `{` is a JSON report — `pane.report_agent` / `pane.report_agent_session`, herdr's real method names — answered in place with one JSON reply (`{"id":…,"result":"ok"}`), and such connections are never registered for broadcasts (registration waits for a connection's first control command, so the send-one-line/read-one-reply/close pattern herdr's scripts use gets exactly its reply). Accepted reports write `agent`/`agent_state`/`agent_session_id`/`agent_session_path`/`agent_source`/`agent_seq` into `MuxPane::metadata` and broadcast `%agent-state-changed <pane> <agent> <state> source=hook` (new `TmuxNotification` variant; unupgraded clients ignore unknown `%` lines); reports at or below the pane's stored `agent_seq` are dropped with no write and no broadcast (herdr's ordering rule), and `unknown` is never written — a hookless agent simply never appears. Panes are seeded with `PAR_MUX_PANE_ID`, `PAR_MUX_SOCKET`, and `PAR_MUX_ENV=1` (herdr's env contract, renamed) so a ported script runs inside any pane, restored panes included. New `AgentPaneFactory` tags `metadata["agent"]` at spawn for embedders and the Phase 6 resume path — the daemon default stays `ShellPaneFactory` (an agent pane is just a command pane until a hook claims it), and hook INSTALLATION (writing the agent's own config) is out of scope by design. `list-agents` replies the roster: one `%N <agent> <state> <source>` line per state-carrying pane (`source` is `hook` or `scrape`), sorted by pane id, stateless panes absent, fixed shape with no `-F`. Authorization is the socket's existing 0600 owner-only boundary. Metadata is deliberately NOT in the save format yet (seam S2's assigned persist gap, Phase 6's entry criterion); `tests/mux_agents.rs` drives the full arc — ported script reports working→blocked, a second client observes both broadcasts, `list-agents` agrees, restart serves the layout back with an empty roster — and asserts the deferral on the actual save bytes.
- **par-mux Phase 5 scrape tier — fallback state for agents without state hooks (Rust `mux` feature)** (`src/mux/scrape.rs`, `src/mux/patterns/{claude,codex,grok}.toml` new). The owner amendment to Phase 5's hook-only ruling: hooks stay primary and authoritative, and a pane whose agent reports no state hook gets its state read from rendered content — OSC title and screen text matched against per-agent pattern rules ported from herdr (Apache-2.0, credited per file). Scope is settled: **claude, codex, grok; state only** (their session identity already arrives by hook). Precedence is structural, not temporal — a pane that has ever accepted a hook state report is hook-authoritative until it dies, and a quiet hook agent holds its last claim (no aging to unknown, no scrape fallback for a claim); the tick skips hook-authoritative panes outright. The honesty rule survives mechanically: a scrape that matches no rule clears any earlier scrape state and yields no state at all — never `idle` — and the push channel stays claim-only (a cleared guess is not broadcast). Provenance rides every surface: `agent_state_source` metadata (`hook`/`scrape`) plus `agent_state_rule` (which pattern fired — a wrong state is debuggable at a glance), the `source` token on `%agent-state-changed`, and the `<source>` column in `list-agents`; the Python `TmuxNotification` projection gains the matching optional `source` field. Patterns ship bundled (`include_str!`) and a local override file — `<state-dir>/par-mux/agent-patterns/<agent>.toml`, beside the state file — shadows the bundled set for its agent, with the shadow and both versions named in the daemon log; an override that fails to parse or validate falls back to bundled with a warning, and a file for an agent with no bundled set adds it. The scrape runs on the accept loop's existing idle poll every second (no new thread; shutdown works by construction). Regions implemented: `osc_title`, `whole`, `bottom_non_empty_lines(N)`, `top_non_empty_lines(N)`; herdr's engine-v3 regions (prompt markers, horizontal rules) and raw OSC 9;4 progress rules are omitted with reasons recorded in each pattern file header and par-mux.md. New optional `toml` dependency, gated under `mux`.
- **par-mux Phase 6 Task 6.2: per-agent resume table with hook-reported override (Rust `mux` feature)** (`src/mux/agent_resume.rs` new). The resume invocation is never persisted — a stored argv would freeze a vendor CLI shape into a state file newer code later restores — so it is rebuilt at restore time from a table shipping with the binary. `resume_argv(agent, session)` encodes the owner's five supported entries in their three irreconcilable shapes (`codex resume <id>` is a subcommand, `claude`/`grok`'s `--resume <id>` and `pi`'s `--session <value>` are separate arguments, `omp`'s `--resume=<value>` is `=`-joined): claude/codex/grok resolve the session id only, while pi/omp accept the transcript path as well (path-first when both persist, matching what the shipped extensions report). An unknown agent returns `None` — a fresh spawn, not an error. `resume_invocation()` checks the hook-reported `agent_resume_argv` first and uses it verbatim, degrading to the table only when the stored value fails the hook endpoint's own shape guarantees (a hand-edited state file) — fail-safe toward a spawn, never a broken command. Entries are cross-checked against herdr's `plan()` (`src/agent_resume.rs:136`) with divergences recorded in the module docs: par-mux scopes to the five, keys on the agent label rather than `(source, agent)`, returns bare argv without herdr's dedupe key, and prefers the path ref. Restore wiring is task 6.3's.
- **par-mux Phase 6 Task 6.1: agent session identity persists in the save format (Rust `mux` feature)** (`src/mux/persist.rs`; `tests/mux_agents.rs`). `PersistPane` gains one optional `agent_session` (`PersistAgentSession`: agent label, session id, transcript path, source tag, and the reported `resume_argv`), captured from the hook-written pane metadata and skipped when absent so a non-agent pane serializes byte-identically to the pre-change format. The wire contract being id-OR-path, neither id nor path alone gates the capture — whichever the metadata holds travels, so the path-only refs the shipped pi/omp extensions send persist too. Restore writes the identity keys back onto the rebuilt pane's metadata (where task 6.3's hook-first lookup already reads), while state, its provenance, the ordering `seq`, the blocked reason, and `session_start_source` deliberately stay out — a restored pane reports state anew or holds none, which keeps the post-restart roster empty by design. `FORMAT_VERSION` bumps to 2: the added field is a compatible serde read, but the bump keeps the boundary explicit — a v1 file is quarantined and the server starts fresh rather than partially reading it.
- **Hook session reports accept herdr's id-or-path contract and carry the agent's own resume invocation (Rust `mux` feature)** (`src/mux/hooks.rs`; `tests/mux_hooks.rs`). `pane.report_agent_session` identifies a session by EITHER `agent_session_id` OR `agent_session_path` — herdr's `session_ref_from_report` shape, and what the shipped par-term pi/omp extensions actually send (their `currentSessionRef` prefers the path and drops the id); requiring the id error-replied every path-carrying session report those two sent, silently dropping `session_start_source` with it. A new optional `session_resume_argv` — an array of non-empty strings, malformed (non-array, non-string, empty) rejected with an error reply — is stored verbatim as `agent_resume_argv` metadata: the agent's own resume invocation (`["pi","--session","<path>"]`), the exact key Phase 6 task 6.2's per-agent table treats as the hook-first override. A session report that moves the pane to a different session without a fresh invocation clears the stored argv rather than let a restart resume the wrong session. Alongside, the blocked-reason `message` pi/omp already send is stored as `agent_message` metadata (cleared when a later report omits it, whitespace collapsed to one line) and rides the `list-agents` roster line as the rest of the line.
- **par-mux Phase 4 protocol prerequisites (Rust `mux` feature)** (`src/mux/`). Lifecycle broadcasts on every mutating dispatch — `%window-add` (new-session/new-window), `%window-close` (kill-window), `%window-renamed`, `%window-pane-changed` (split/select-pane/select-window/kill-pane), `%session-changed` to the issuing client — so a push client sees window lifecycle instead of inferring it. `send-keys` now speaks the tmux contract: key names (`C-a`…`C-z`, `C-Space`, `Escape`, `BSpace`, `Space`, arrows, `Enter`), `-l` literal payloads, `-H` hex bytes, and bare `0xNN` tokens, with the unconditional newline append removed (Enter is an expressible key, not a grammar workaround). `resize-pane` gains absolute sizing (`-x COLS`, `-y ROWS`, either alone) alongside the relative `-L/-R/-U/-D` form, and both forms — plus `split-window`, `swap-pane`, and `kill-pane` — now resize the affected panes' terminals and PTYs to the layout geometry (previously terminals kept stale sizes and diverged from the layout string on every mutation; a restart re-fits restored panes the same way). Relative resize works for a pane on either side of its bordering split (previously only the split's `first` child). `refresh-client -C WxH` reports a client's renderer size; the window resizes to it under the window-size policy (the latest report wins) and broadcasts a `%layout-change` with the re-divided geometry — without `-C`, `refresh-client -t %N` keeps its screen-replay meaning. Graceful shutdown pushes `%exit` to every connected client. Bare `new-window` (no `-t`) creates in the most-recently-created session, and the fixed list-reply shapes (`%N` lines / `@N: name` / `$N: name`) are the documented query contract — `-F` is deliberately not implemented (push covers what it existed for).
- **par-mux Phase 2 command set (Rust `mux` feature)** (`src/mux/`). Pane commands — `split-window` (`-h`/`-v` orientation, `-p` percent for the new pane, reply carries the new pane id), `select-pane`, `resize-pane` (`-L`/`-R`/`-U`/`-D` with cell count, default 5), and `swap-pane` (`-t`/`-s`) — each mutating the window's binary `LayoutTree` and broadcasting a `%layout-change` notification to every connected client, rendered by the Task 2.2 layout-string emitter. `capture-pane -S`/`-E` now select an exact line range under tmux's offset convention (`0` = first visible line, negative = history lines back from the screen top) instead of a tail count, and `MuxClient` keeps — and can `kill_spawned_daemon()` — the daemon child it spawns.

### Changed
- **The alternate screen no longer reflows on resize (Rust + Python API)** (`src/grid/scroll.rs`, `src/terminal/mod.rs`). `Terminal::resize` reflowed the alt grid exactly like the main grid, so any width change joined or split a full-screen app's rows; full-screen TUIs redraw the alt screen on SIGWINCH (often only the cells they believe changed — ratatui, curses), so reflowed cells never got repainted and the scramble persisted. Measured: a 154×48 TUI went from 0 wrapped rows to 22 after one resize, producing 308-column joined rows and garbled layouts in par-mux panes after any resize or reattach. `Grid::resize_without_reflow` truncates or pads each row in place, keeping row positions and clearing wrap flags; `Terminal::resize` uses it for the alt grid only — main-grid reflow (and `resize()`'s documented behavior for the primary screen) is unchanged. xterm and tmux leave the alt screen un-reflowed the same way.
- **par-mux is now a clap CLI, gains `--help`/`--version`, and `web_term/`/the derive crate are excluded from the crate packaging (Rust)** (`src/bin/par_mux/main.rs`, `Cargo.toml`, `.parsightignore`; ARC-012/ARC-014/ARC-017). `par-mux`'s hand-rolled argument parsing is replaced with `clap::Parser`, so `--help` and `--version` now work (previously there was no `--help` at all); the daemon binary gained `--stop`, `--restart`, and `--state-dir` as part of the same change (see the `kill-server`/`--stop`/`--restart` entry above). `par-term-emu-derive` is now a Cargo workspace member instead of a path dependency built separately, and the generated Next.js static export (`web_term/`) is excluded from the published crate package and from parsight's analytics (`.parsightignore`) — neither changes runtime behavior.
- **`export_scrollback` format `"ansi"` now preserves SGR (Python API + Rust)** (`src/terminal/semantic_snapshot.rs`, `src/grid/export.rs`). The `ExportFormat::Ansi` arm previously fell through to the Plain text export, silently stripping colors and attributes; it now emits per-run SGR (the styled counterpart of the Plain path — same line selection and order, `max_lines` honored). Output changes for callers who passed `format="ansi"` and expected plain text: escape bytes are now present.
- **par-mux state persistence no longer blocks control traffic (Rust `mux` feature)** (`src/mux/persist.rs`, `src/mux/server.rs`, `src/mux/dispatch.rs`, `src/mux/pane.rs`; ARC-003). A mutating command no longer serializes every pane's scrollback and fsyncs while holding the tree lock: dispatch captures the `PersistState` under the lock (cheap clones, with a per-pane snapshot cache keyed on the PTY generation and terminal size so an idle pane costs one `Vec<Cell>` clone) and hands it to a single persist worker thread that coalesces bursts to the newest state before writing. Structural-command latency no longer scales with total scrollback. A clean shutdown joins the worker before the existing synchronous final save, so the on-disk format and the "clean SIGTERM never loses the last window" guarantee (D3.3) are unchanged.
- **Headless build profiles pull fewer dependencies (Rust)** (`Cargo.toml`, `src/lib.rs`, `.github/workflows/ci.yml`; ARC-004/ARC-005). The `mux` feature no longer enables `tokio` (no mux code uses it), and `subtle`/`zeroize` are optional deps owned by the `streaming` feature; the `streaming` module root compiles only for `streaming`/`python`/`python-test`, so `sim`/`rust-only`/`mux` builds no longer compile the protocol layer. The CI D1 tree guard now asserts none of tokio/portable-pty/subtle/zeroize/prost/axum reaches `sim`. A `sim` consumer that referenced `streaming::protocol` types would now fail to compile — vendor with `features = ["streaming"]` if needed.
- **DECSET/DECRST share one mode table, and DECRQM reports every implemented mode (Rust)** (`src/terminal/sequences/csi/mode.rs`, `src/terminal/sequences/csi/report.rs`; ARC-009/QA-105). The four parallel mode-table matches collapse into `dec_mode_label` (read side), `set_dec_private_mode` (write side, asymmetric modes explicit), and a shared `emit_mode_changed` tail; a set/reset/report symmetry test pins the table. Behavior notes: DECRQM previously answered "not recognized" (`;0$y`) for implemented modes 69, 1004, 1005/1006/1015 — it now reports their real status — and a redundant `CSI ? 80 h` no longer emits a duplicate `ModeChanged` event (the old table dropped mode 80 from its changed-check, making every DECSET 80 emit).
- **par-mux logs through `log` instead of raw stderr (Rust `mux` feature)** (`src/mux/*.rs`, `src/bin/par_mux/main.rs`; ARC-010). Library messages route through `log::warn!`/`log::error!` by severity, and the daemon installs a minimal stderr logger so they stay visible; library embedders see nothing until they install their own logger.
- **A stalled par-mux control client is evicted and disconnected, not merely de-registered (Rust `mux` feature)** (`src/mux/server.rs`; ARC-011 + ENH-012). Per-client broadcast queues are bounded at 4096 lines (worst case ~128 MiB — a line carries one PTY read, up to 16 KiB raw roughly doubled by escape encoding). A client whose queue fills is evicted, tmux's policy for a control client that stops draining; a draining sibling provably loses nothing. Eviction also tears the connection down: the queue sender drop alone left the wedged client's writer thread blocked in a full socket buffer, its ~queued lines retained, and the socket open — the client never learned it was evicted and the daemon carried the zombie indefinitely (measured live: RSS held ~70 MiB and the socket never EOF'd for 100+ seconds of continued flood). Connection threads now run with short send/recv timeouts and poll an eviction flag, so a wedged client is fully disconnected — queue freed, socket closed — within about half a second; healthy slow consumers are unaffected (a timeout wake mid-line or mid-write retries with the partial preserved).
- **Python binding dedup: 17 duplicated methods fold into one macro (Python API)** (`src/python_bindings/common.rs`, `pty.rs`, `terminal/*`; ARC-007/QA-106/QA-107). The exports/stats/snapshot/keyboard-flag surface is written once against `TerminalAccess` and emitted for both `Terminal` and `PtyTerminal`; unified on the richer of each drifted pair — `create_snapshot` resolves ANSI-palette colors with bold brightening on both, `get_char` returns the grapheme cluster (was the base char on `PtyTerminal`), `get_stats` carries the full key set, and the recording exports take `session=None` on `PtyTerminal`. `resize`/`resize_pixels` now take `int` clamped the same way on both classes (was u16-ranged on `PtyTerminal`). A panicking par-mux command is contained per-connection (QA-113): the issuer receives an `internal error` block and the connection survives.
- **`poll_events` event dicts carry native Python types — breaking (Python API)** (`src/python_bindings/observer.rs`, `src/python_bindings/terminal/mod.rs`; QA-119). Numeric fields (`cols`, `rows`, `row`, `col`, `end_col`, `first_row`, `last_row`, `zone_id`, `abs_row_start`/`abs_row_end`, `depth`, `trigger_id`, `volume`, `percent`, `id`, `timestamp`, `exit_code`, `cursor_line`, `total_bytes`, `bytes_transferred`, `size`) are now `int`; boolean fields (`enabled`, `include_scrollback`) are `bool`; optional fields (`old_value`, `old_cwd`, `filename`, `id`, `command`, `exit_code`, `label`, `state`, `badge`, `shell_type`, `old_hostname`, …) are present as `None` when unset instead of omitted. Applies to `poll_events()`, `poll_subscribed_events()`, and observer callbacks (`add_observer` / asyncio queue). Migration: switch comparisons and `int(...)`/`str(...)` coercions to the native values, or call the new `poll_events_legacy()` / `poll_subscribed_events_legacy()` — byte-identical to the old output (every value a `str`, bools `"true"`/`"false"`, unset optional keys omitted) — while migrating; the legacy methods are kept for one release. Still stringly-typed (unchanged): `poll_cwd_events()`, `poll_shell_integration_events()`, `get_named_progress_bar()`/progress-bar query methods, and `drain_bell_events()`.
- **Scroll-path performance: ~3x faster on the memmove-class hot paths (Rust; no observable behavior change)** (`src/grid/scroll.rs`, `src/terminal/write.rs`; ENH-010). `scroll_up`/`scroll_region_up`/`scroll_down`/`scroll_region_down` moved rows via per-cell `clone_from_slice` (each `Cell` carries a `SmallVec` combining-char buffer, so the clone dominated runtime); they now relocate rows with `Vec::drain`/`rotate_right`-class memmoves, and printable ASCII with the plain charset active skips the Unicode/width/normalization machinery in `write_char`/`print`. Interleaved A/B measured 3.1x on `plain_ascii`/scroll (`docs/fable/BENCH-BASELINE-2026-09.md`); output is byte-identical, pinned by the existing scroll and print test suites plus new inline/spilled-scrollback regression tests.

### Fixed
- **par-mux reattach seed now replays scrollback, not just the visible screen (Rust `mux` feature)** (`src/grid/export.rs`, `src/mux/dispatch.rs`, `src/terminal/semantic_snapshot.rs`). A reattached client pane was seeded from `export_screen_restore_sequence` alone, which carried only the visible screen — the client emulator started with no history, so the scrollbar had nothing to show and the mouse wheel could not scroll back. The restore sequence now writes the main screen's scrollback first, top-down from the home position (`\x1b[H`, each line CR-led and LF-terminated so it lands in the replay target's scrollback in order — the sequence is no longer `\n`-free the way the original capture-pane `-e` entry above describes; see `docs/MUX.md`'s corrected wire-shape note), then pushes the lines still on screen off from the bottom row before the styled content block. Under a full-screen app the main screen is written before the alt-screen switch, so quitting the app returns to the shell's screen (with history) instead of a blank one.
- **A restarted par-mux daemon returns a full-screen pane to the main screen instead of the frozen alt screen (Rust `mux` feature)** (`src/mux/persist.rs`, `src/terminal/replay_snapshot.rs`). A pane saved while a full-screen app (htop, top) held the alternate screen was restored with the app's alternate screen, scroll region, and input modes still active — but under a freshly spawned shell. The shell's output then went to the alternate screen, which has no scrollback, so the pane never accumulated history after a daemon restart. Restore now uses `restore_for_new_process`, which keeps the saved screen content and scrollback but drops the state the old process set up for itself: back on the main screen with the alternate grid cleared, the scroll region and margins reset, and cursor-key/keypad/mouse/focus/keyboard-protocol modes off.
- **A split pane's output now reaches clients — new split panes previously rendered blank (Rust `mux` feature)** (`src/mux/dispatch.rs`; `tests/mux_daemon.rs`). `cmd_split_window` created the pane through `split_pane_in_window` but never installed its output sink, unlike `new-session` and `new-window`. The pane's PTY fed the daemon grid (`capture-pane` showed the prompt), yet no `%output` line was ever pushed, so every client rendered the new split pane blank. Present since the ARC-002 dispatch decomposition.
- **par-mux command grammar accepts quoted session and window names with spaces (Rust `mux` feature)** (`src/tmux_control.rs`, `src/mux/command.rs`). `new-session -s` and `new-window -n` read their value from the pre-split token vector, so a name containing a space was truncated at the first whitespace (`new-session -s Par Mux Test` silently created a session named `Par`), and quoting could not work around it because the split happened before the flag scan. Those two name flags now route through the same bounded quoting grammar `send-keys` payloads already use.
- **par-mux fails fast when the daemon binary cannot be started, instead of a silent 10 s retry (Rust)** (`src/mux/client.rs`). `connect_or_spawn_at` treated "no daemon was started" and "the daemon is slow to bind" as the same outcome — a missing `par-mux` binary sent the caller into the full `SPAWN_CONNECT_DEADLINE` retry loop, reconnecting to a socket nothing would ever bind, then surfaced a generic connect error with no hint the binary itself was absent. Spawn now propagates the actual `io::Error` (measured: 10.05 s to learn nothing, versus 377 µs now).
- **IL/DL with a count past the scroll-region size no longer panics (Rust)** (`src/grid/edit.rs`; found by the new ENH-014 fuzz target within its first minute). `Grid::insert_lines` clamped the line count against the unclamped `scroll_bottom` and then underflowed its copy-range endpoint: `CSI 24 L` (or any larger count) at the region top panicked in debug builds and wrapped into an immediate out-of-bounds access in release; a `scroll_bottom` past the screen triggered the same underflow for any large count. `delete_lines` had the twin defect — its shift read one row past the region in the same edge. Both now clamp against the clamped region bottom and iterate by destination so no endpoint can underflow; four regression tests pin the exact edges (grid-level and a terminal-level `CSI 65535 L`/`M`).
- **par-mux exits promptly on SIGTERM with a scrollback-heavy pane (Rust `mux` feature)** (`src/mux/persist.rs`). Two compounding causes in the final synchronous shutdown save: `write_state` encoded through an unbuffered `File` (one write syscall per JSON fragment) and the persisted snapshot carried every pane's full scrollback. Measured live: a pane with a maxed 10 000-line scrollback serialized to a 136 MB state file and the daemon took 122 s to exit after SIGTERM (control, no flooded pane: ~0.1 s). The encoder now writes through a `BufWriter`, and the persisted snapshot caps per-pane scrollback at the newest 100 000 cells (~1 250 lines at 80 cols). The cap is a persistence bound only — the running pane keeps its full in-memory history, a restored pane still grows to its configured depth, zones are dropped/clamped at the cap floor exactly as live scroll-out eviction does, and the on-disk format is unchanged (v2).

## [0.49.0] - 2026-09-19

### Added
- **`export_asciicast_v3()` exports recordings in asciicast v3 format with graphics events** (`src/terminal/recording.rs`, `src/python_bindings/terminal/recording_api.rs`, `src/python_bindings/pty.rs`, `src/graphics/mod.rs`, `src/graphics/serialization.rs`). The v3 export uses the nested `term` header object, relative per-event intervals, and `"COLSxROWS"` resize strings per the asciicast v3 spec, and adds a `g` graphics event (the spec's code list is open; readers must skip unknown codes) per graphic that entered the store during the recording — live placements plus scrollback promotions — carrying protocol, geometry, position, and base64 RGBA pixels that the text-only stream cannot convey. Graphics now carry an `added_at` unix-ms store-entry stamp (`TerminalGraphic` + `SerializableGraphic`, serde-defaulted for older exports) used to time the events. The v2 export is unchanged (byte-shape pinned by tests). Python bindings on both `Terminal` and `PtyTerminal`; API_REFERENCE updated.
- **Parameterized XTPUSHCOLORS/XTPOPCOLORS slot forms are now implemented** (`src/terminal/sequences/csi/color_stack.rs`, `src/terminal/sequences/csi/mod.rs`, `src/terminal/mod.rs`). `CSI Pi # P` stores the current dynamic + ANSI palette colors into stack slot Pi (1–10) without pushing, and `CSI Pi # Q` restores slot Pi without popping; Pi 0 or omitted keeps the push/pop semantics, and XTREPORTCOLORS (`CSI # R`) depth/high-water reporting is unchanged. Storing into a slot beyond the current depth grows the stack, padding intermediate slots with snapshots of the current colors (a snapshot stored into an empty stack's slot 1 therefore becomes the pop target). Previously parameters were silently ignored and the bare form applied. Stale "not implemented" color-stack notes in `docs/VT_TECHNICAL_REFERENCE.md` (pre-dating the 0.47.0 ship) are corrected alongside.
- **DECSDM sixel display mode (`CSI ? 80 h/l`) is now implemented** (`src/terminal/mod.rs`, `src/terminal/sequences/csi/mode.rs`, `src/terminal/sequences/csi/report.rs`, `src/terminal/sequences/dcs/mod.rs`). Previously mode 80 fell through to the unsupported-DECSET log; now set (display mode) paints sixel graphics at the home position without scrolling or moving the cursor, while reset (scrolling mode, the default and unchanged behavior) paints at the cursor with the existing cursor advance. DECRQM reports mode 80, and DECSTR/RIS restore the default. Legacy sixel emulators that assume display placement (xterm/mlterm/WezTerm semantics) now position correctly.
- **Kitty placements now advance the cursor below the image** (`src/graphics/kitty.rs`, `src/terminal/mod.rs`). A display placement (`a=T`/`a=p`) at the cursor moves it to the first line below the image, spanning the placement's row count via the ordinary newline path — the scroll region and scrollback promotion apply exactly as for a multi-row text write. The `C=1` key suppresses the move (previously `C=` was silently dropped by the parser and the cursor never advanced, which broke layout for emitters like chafa); `C=0` or an omitted `C=` keeps the default, and virtual placements (`U=1`) never move the cursor, matching kitty's own rule for unicode placements.
- **The web terminal frontend renders Sixel and iTerm2 inline graphics** (`web-terminal-frontend/components/Terminal.tsx`, `web_term/`). `@xterm/addon-image` is loaded so raw Sixel `DCS q` and iTerm2 `OSC 1337` image sequences forwarded by the existing output stream render inline in the browser, and the addon enables the standard image size reports. Kitty APC pixel placements remain unsupported by xterm.js (no official Kitty image addon or pixel overlay transport). No streaming-protocol change is required.

### Changed
- **Dependency refresh**: Python dev floors synced to their resolved versions (`uv.lock`; dev-dependencies only) and the web frontend toolchain refreshed, holding ESLint on 9.x. No runtime Rust dependency changes.

## [0.48.0] - 2026-08-30

### Added
- **Kitty placement geometry metadata is preserved** (`src/graphics/kitty.rs`, `src/graphics/mod.rs`, `src/graphics/serialization.rs`). Lowercase `x=`/`y=`/`w=`/`h=` are now parsed as the Kitty source crop rectangle — new `source_x`/`source_y`/`source_width`/`source_height` fields on `ImagePlacement`, included in the JSON graphics export with serde defaults so older exports still deserialize — while uppercase `X=`/`Y=` are the within-cell display offsets. Previously the parser misread lowercase `x=`/`y=` as cell offsets, losing the crop rectangle entirely (and double-reading the same values as animation frame offsets); `d=x`/`d=y` delete targets now key off the source rectangle as the spec defines. Redisplaying an explicit nonzero `(i=, p=)` pair is an upsert: `GraphicsStore::add_graphic` replaces the existing placement in place instead of appending a duplicate, while a zero or omitted placement ID remains intentionally non-unique and coexists.

### Changed
- **`TerminalGraphic::height_in_rows` takes `(fallback_cell_width, fallback_cell_height)`** (`src/graphics/mod.rs`; previously `height_in_rows(cell_height)`). It now delegates to `cell_span` so the omitted-axis aspect-ratio logic is shared between the two; callers that don't know the cell width pass 0 and the stored `cell_dimensions` or pixel-derived fallback takes over. **Breaking for Rust `rlib` consumers only** — the method is not exposed to Python.

### Fixed
- **Kitty APCs now process in stream order with interleaved cursor movements** (`src/terminal/apc_filter.rs`, `src/terminal/mod.rs`). The APC pre-filter previously collected all completed Kitty payloads and processed them after the entire passthrough byte stream was fed to the VTE parser. This meant `CSI row;col H` cursor moves emitted by clients like Herdr before each `a=p` placement were applied only after all placements had already been built, stacking every image slice at a single terminal position. The filter now records the passthrough offset at each APC completion and the terminal advances each intervening passthrough slice before building that graphic, so slices land at their intended consecutive rows. Regression test added covering interleaved `CSI H` + APC pairs asserting distinct graphic rows.
- **Kitty `d=` delete commands resolve after the full key=value list** (`src/graphics/kitty.rs`). Emitters disagree on key order — Herdr sends `a=d,d=I,i=<id>` with the identifying parameters *after* the delete spec — and resolving while scanning saw `image_id`/`placement_id` still unset, silently no-opping the delete. Resolution now runs after the whole list is parsed (and re-resolves as later chunks accumulate parameters), and `d=I` accompanied by `p=` now targets that specific placement instead of every placement of the image.
- **Kitty placement footprints (`c=`/`r=`) govern row spans and scrolling** (`src/graphics/mod.rs`, `src/terminal/mod.rs`). `cell_span` and `height_in_rows` now honor an explicit footprint over pixel-derived sizes, and every row-query and scroll path routes through the shared computation, so a placement rendered into `c=2,r=1` cells occupies exactly one row for `graphics_at_row`, scroll retention, and scrollback promotion instead of the three rows its native pixels imply. The terminal stamps its cell dimensions onto ingested Kitty graphics so pixel-derived fallbacks use the real cell metrics.
- **An omitted `c=`/`r=` axis is computed from the cropped source aspect ratio** (`src/graphics/mod.rs`). When only one footprint axis is specified, the other is derived from the effective source dimensions — the crop rectangle (`x=`/`y=`/`w=`/`h=`) when present, clamped to the image bounds — so partially-specified placements render undistorted per the Kitty graphics protocol instead of falling back to full-image pixel math. A fully-unspecified footprint also derives from the cropped source, and the computed axis is never zero.
- **Retransmitting a Kitty image ID deletes its previous placements** (`src/graphics/mod.rs`). The Kitty spec (lines 547–550) makes retransmit of an existing `i=` a delete of the old image and all its placements; the store previously kept them, so a transmit → display → retransmit → display cycle (e.g. Herdr replaying its session) accumulated duplicate placements. Retransmit now clears the image's active placements, virtual placements, scrollback entries, and animation state before storing the new data.

## [0.47.0] - 2026-08-28

### Security
- **Streaming-server HTTP origin hardening** (`src/streaming/server.rs`). With no `--allowed-origins` allowlist configured, HTTP CORS previously fell back to `CorsLayer::very_permissive()` while the WebSocket policy default-denied remote browsers; the no-allowlist branch now mirrors the WS local-origin default via `AllowOrigin::predicate` over the shared host classification. The `/sessions` HTTP endpoint gained the same `check_ws_origin` guard the WS handlers use (403 for disallowed browser origins; no-origin native clients unaffected).
- **Kitty file-transmission path check is now component-wise** (`src/graphics/kitty.rs`). The old `contains("..")` substring test wrongly rejected filenames like `my..notes.png`; rejection now keys on real `Component::ParentDir` elements. `docs/SECURITY.md` states plainly that absolute paths are readable by design and the `..` check is not a sandbox.
- **Debug log temp-file hardening** (`src/debug.rs`). PID-suffixed filename, `0600` permissions and `O_NOFOLLOW` on Unix; a symlink at the path now fails closed.

### Added
- **XTPUSHCOLORS/XTPOPCOLORS/XTREPORTCOLORS color palette stack** (`src/terminal/sequences/csi/color_stack.rs`, `src/terminal/mod.rs`, `src/terminal/sequences/csi/mod.rs`). `CSI # P` snapshots the dynamic colors (OSC 10/11/12) plus the ANSI palette (OSC 4) onto a 10-deep stack; `CSI # Q` restores the top entry; `CSI # R` replies `CSI ? used ; last # Q` (current depth and high-water mark, matching xterm 397 byte-for-byte). Previously `CSI # P` misrouted to DCH and deleted characters; bare `CSI P` remains DCH. Pushes beyond the cap are ignored, pop on empty is a no-op, RIS/DECSTR clear the stack, and parameterized store/restore forms (`CSI Pi # P/Q`) are not implemented (parameters ignored). Docs rows restored in `docs/VT_SEQUENCES.md` and `docs/VT_TECHNICAL_REFERENCE.md`.
- **X10 mouse tracking (`CSI ? 9 h/l`, DECSET/DECRST 9) is now wired** (`src/terminal/sequences/csi/mode.rs`, `src/terminal/mod.rs`). `MouseMode::X10` existed but no DECSET arm ever set it, so legacy apps requesting mode 9 got nothing. `report_mouse` now applies X10 semantics: button presses only — release and motion (button 3) produce no report — with modifier bits stripped from the legacy `CSI M CbCxCy` encoding. Setting 9 and 1000 interact like the 1000-series modes (most recent set wins), and mode 9 is reported by DECRQM. Supersedes audit findings DOC-003/QA-007's deliberate leave-in-place.
- **DECSACE (`CSI Ps * x`) is now implemented** (`src/terminal/sequences/csi/window.rs`, superseding QA-010's parsed no-op). `Ps` 0/1 selects stream extent — DECCARA/DECRARA then change everything in reading order from the start corner to the end corner — and `Ps` 2 selects strict rectangle extent. The no-spurious-DECREQTPARM-reply guarantee from QA-010 is retained. The terminal starts in rectangle extent and DECSTR/RIS restore it, preserving this library's shipped rectangle-only behavior; VT420 powers on in stream mode, so a fidelity-following change of the default would silently alter existing DECCARA output and was deliberately not taken.
- **`OSC 1337;CurrentDir=<path>` is now supported** (`src/terminal/sequences/osc/iterm.rs`). iTerm2's working-directory report previously fell through to the inline-image handler and was rejected; it now performs the same state update as OSC 7 (tracked cwd, `CwdChanged` event, badge session variables). The path arrives raw (no `file://` URL), and because the sequence carries no host identity, the previously recorded hostname/username are preserved. The `accept_osc7` security gate applies to this alias just as it does to OSC 7.
- **Python type stubs: `py.typed` + generated `_native.pyi`** (`python/par_term_emu_core_rust/`, `scripts/generate_stubs.py`). The package advertises `Typing :: Typed` honestly now: 69 classes, 1,335 methods, 342 properties (setters render as properties), 27 module functions. A `make stub-check` target is wired into `checkall` and CI.
- **Web frontend test harness** (`web-terminal-frontend/`). vitest + happy-dom with 35 tests over `lib/protocol.ts` (encode/decode round-trips, framing, size guards) and the new `TerminalConnection` (reconnect backoff, heartbeat/stale-pong, shutdown classification, idempotent dispose); `make test-web` runs in `checkall`.
- **Legacy alternate-screen modes 47, 1047 and 1048 are now implemented** (`src/terminal/sequences/csi/mode.rs`, `src/terminal/mod.rs`, superseding DOC-003's leave-in-place). Only 1049 was wired, so apps hardcoding the legacy modes got no screen switch at all. Mode 47 switches to the alternate screen preserving its contents; 1047 clears the alternate screen on both entry and exit; 1048 is a DECSC/DECRC-equivalent cursor save/restore with no screen switch. The 1049 arm was factored into shared `enter_alt_screen`/`exit_alt_screen` helpers (1049 behavior unchanged and pinned by its regression tests). Alt screens do not nest — setting any of 47/1047/1049 while already alternate is a no-op, and any of the three resets returns to primary. All three modes are reported by DECRQM (1048 reports whether a saved cursor is pending restore).
- **`Terminal.diff_snapshots()` is now implemented** (`src/terminal/semantic_snapshot.rs`, `src/python_bindings/terminal/badge_api.rs`, ENH-001). The `SnapshotDiff` class was registered but no binding produced one, and the previously documented `diff_snapshots()` raised AttributeError (DOC-004 removed the stale entry; this restores it with the real signature). A new core `diff_snapshots(&SemanticSnapshot, &SemanticSnapshot)` splits each snapshot's newline-joined `visible_text` into lines and delegates to the existing `diff_screen_lines`; the Python method takes two `get_semantic_snapshot()` dicts and returns a `SnapshotDiff`. Row-by-row semantics: because `content()` pads blank rows, a blank row gaining content is reported as modified rather than added. Raises `ValueError` when a dict lacks a `visible_text` string.
- **Criterion VTE throughput benchmark suite** (`benches/terminal_throughput.rs`, `Cargo.toml`, `Makefile`, `docs/fable/BENCH-BASELINE-2026-08.md`, ENH-007). Drives the real `Terminal::process` pipeline (APC pre-filter, vte parser, sequence dispatch, grid writes, scrolling, Sixel DCS / Kitty APC graphics ingestion) with deterministically generated payloads across seven groups — `plain_ascii`, `sgr_heavy`, `unicode_wide`, `scroll`, `cursor_addressing`, `sixel_decode`, `kitty_decode` — reporting MiB/s via `Throughput::Bytes`. Criterion 0.7 is a dev-dependency with the `[[bench]]` target gated on `required-features = ["rust-only"]`, so everyday `cargo test` and `make checkall` runs skip the target entirely; `make bench` runs it on demand. A committed baseline (Apple M4 Max, rustc 1.98.0) records text paths at 3.4–17 MiB/s (newline-scroll churn as the floor), sixel at 71.6 and kitty at 667.8 MiB/s, and `CONTRIBUTING.md` gained a Benchmarks section covering `--save-baseline`/`--baseline` comparison.

### Changed
- **MSRV raised 1.90 → 1.98** (`Cargo.toml`; docs synced in `README.md`, `docs/BUILDING.md`, `docs/CROSS_PLATFORM.md`, `docs/MATURIN_BEST_PRACTICES.md`). Aligned to the current stable toolchain (rustc 1.98.0). **Breaking for Rust consumers**: the `rlib` now requires Rust ≥ 1.98; PyPI (Python wheel) users are unaffected (CI builds wheels with the stable toolchain). Rust's Linux glibc floor is unchanged at 2.17 (manylinux2014).
- **Streaming codec conversions are macro-generated** (`derive/src/lib.rs` `PyDictConvert`, `src/streaming/py_convert.rs`). The four Python codec functions in `src/python_bindings/streaming.rs` are thin wrappers over generated conversions (file shrank 2,077 → 1,082 lines; `decode_server_message` complexity 240 → 2); the dict format is pinned by 75 characterization tests that passed identically before and after, and all matches over protocol enums are now exhaustive (no `_ =>`). The five `Connected` constructors were replaced by a `ConnectedBuilder` (partials kept as deprecated shims one release).
- **`src/streaming/server.rs` decomposed** into `config.rs` / `session.rs` / `rate_limit.rs` (4,224 → 2,741 lines) with all types re-exported at their old paths; the streaming `SessionState` was renamed `StreamSessionState` to end the collision with the multiplexing one. The `par-term-streamer` binary split into `cli.rs` / `frontend_download.rs` / `bootstrap.rs` (main.rs 1,790 → 572 lines) and its three user-reachable `.expect` panics are now early usage errors.
- **Dependency hygiene**: archived `serde_yaml` replaced with `serde_yaml_ng` (clears RUSTSEC-2024-0320); `tokio` trimmed from `full` to the eight features actually used; `exr`/`hdr`/`dds` decoders dropped from `image` (clears the `exr` → `pulp` → `paste` edge of RUSTSEC-2024-0436; `cargo tree -i paste` is empty).
- **`web_term/` is deterministic and CI-gated**: the Next.js build ID is pinned to the package version (consecutive builds are byte-identical) and a CI job fails on `git diff --exit-code web_term/`, naming `make web-build-static` as the fix. `bun.lock` is the single canonical frontend lockfile (`package-lock.json` removed).
- **Library diagnostics route through the `log` facade** instead of production `eprintln!` (observer callbacks, terminal loop); no logger is initialized — that remains the embedder's choice.
- **Frontend decomposition**: connection lifecycle extracted from `Terminal.tsx` into a framework-free `lib/terminal-connection.ts`; static key-layout data moved to `lib/keyboard-layouts.ts`.

### Fixed
- **`tests/test_streaming.py` passes again under the streaming build** (28/28 including the slow stress tests; previously the four websocket tests errored at setup, two protocol assertions failed, and the run aborted at the suite's 5s per-test timeout). The `streaming_server` fixture is now a proper `pytest_asyncio.fixture` — strict mode (the default) leaves plain async-generator fixtures unhandled, erroring every requesting test at setup. Received frames are asserted as binary protobuf (compression-flag byte + payload), not `str`. The initial-screen tests decode the `connected` message and assert on its `initial_screen` field instead of racing a recv timeout (a client always receives `connected`, so "nothing arrives within 0.3s" was wrong about the protocol). Test clients connect with `close_timeout=1`: the server keeps flushing queued output before completing the close handshake, so the websockets default (10s) blew the per-test timeout on tests that close with unread messages.
- **Kitty placement APCs now store decoded graphics** (`src/terminal/mod.rs`). Regular Kitty placement commands returned a decoded `KittyGraphicResult::Graphic`, but the terminal integration discarded it after the image upload, leaving placements with no active pixel-backed graphic for downstream renderers. The result is now inserted into `GraphicsStore`; a transmit followed by placement renders with its stored pixels. Regression coverage added in `src/terminal/tests/kitty_apc.rs`.
- **`get_word_at`/`select_word` are display-column correct** (`src/text_utils.rs`, `src/terminal/screen.rs`). The old implementation confused display columns with char indices and byte lengths, returning wrong words (or none) on CJK/emoji/multi-byte lines; both now walk grid cells (spacer columns resolve to their wide char). Behavior notes aligned with `docs/API_REFERENCE.md`: the default word set is iTerm2's `/-+\~_.` and non-word characters return `None`.
- **`CSI Ps * x` (would-be DECSACE) no longer emits a spurious DECREQTPARM reply** (`src/terminal/sequences/csi/mod.rs`); it is consumed as a parsed-but-unimplemented no-op while bare `x` still replies.
- **Achromatic HSL scale bug**: Python `rgb_to_hsl((255,255,255))` returned lightness 1.0 instead of 100.0 (`color_utils.rs` early-return used the 0–1 scale); pixel/color helpers in the bindings now delegate to canonical core implementations.
- **Kitty zlib empty-input contract restored** (`src/graphics/kitty.rs`): a dependency bump made zero-length input return `Err` from `flate2`; empty payloads are empty output again.
- **Documentation synced to reality** (18 issues): QUICKSTART build commands now run (`streaming-bin` features, port 8099, MSRV 1.98); every `docs/RUST_USAGE.md` recipe compile-verified in scratch crates; `API_REFERENCE.md` signatures that raised `TypeError`/`AttributeError` corrected and every snippet executed; `VT_TECHNICAL_REFERENCE.md` false implementation claims flipped (charset support documented as implemented); README install refs and 975 lines of duplicated changelog archived; binding-path and threading-doc drift fixed in `CLAUDE.md`/`CONTRIBUTING.md`.
- **Build tooling**: the Makefile `lint`/`typecheck`/`clippy` targets and the pre-commit clippy hook no longer pass `--all-features` (which selects the invalid `sim`+`python` pair now rejected by a `compile_error!` guard). The deployment workflow's macOS wheel job now installs a Rust toolchain explicitly (maturin-action only runs `rustup target add` against the runner's preinstalled rustc, which at 1.97.1 is below the 1.98 MSRV), and the wheel-test jobs install `pytest-asyncio` (a module-level import in `tests/test_streaming.py` since its fixture conversion). The pre-commit whitespace fixers now exclude generated `web_term/`, which must stay byte-identical to a fresh build for the CI determinism gate.
- **Asciicast v2 export timestamps were 1000× too small** (`src/terminal/recording.rs`). `record_event` stores milliseconds since recording start, but the v2 export divided by 1_000_000 assuming microseconds, shrinking every event delay 1000× so exported casts replayed essentially instantaneously. The export now divides by 1_000, and the struct doc comments that claimed microseconds (the source of the mistake) are corrected. Regression test `test_export_asciicast_timestamp_scale`: an event recorded after a 50ms sleep exports at ~0.05s, not 0.00005s.
- **WebSocket closing handshake completed on both server paths** (`src/streaming/client.rs`, `src/streaming/server.rs`). When a client disconnected, both `run_ws_session` (tungstenite) and `handle_axum_websocket` (the axum HTTP+WS path serving `web_term/`) broke their session loops on Close and dropped the stream without sending a Close reply — the TCP connection ended with a bare FIN and clients saw abnormal closure (1006) instead of completing the RFC 6455 closing handshake. `Client::close` now handles both directions (when the peer initiated the close, tungstenite has already queued our reply in `do_close()` and the further send is rejected as `SendAfterClosing`, so it falls back to `flush()`, which writes the queued reply without the write-path state check); the axum handler mirrors the same best-effort Close-then-flush after its loop. Regression tests connect and assert `close_code` 1000 (echoed handshake); without the fix both observe 1006. Investigation of a reported ~10s close stall showed the server was never holding the connection (the session loop breaks on Close within ~0.7s even under heavy output) — the remaining wait is websockets-client assembler backpressure, which stalls identically against websockets' own reference server.

### Removed
- **Dead code on the `rlib` surface** (verified unreferenced in this repo, its tests/examples, and the sibling consumers `par-term` and `par-term-emu-tui-rust`). **Breaking for Rust consumers** who called these directly; Python users are unaffected (none were exposed to Python): `Cell::new_with_config`, `Cell::with_colors_and_config`, `Cell::from_grapheme_with_config`, `Cell::from_grapheme_normalized`, `Cell::recalculate_width` (`src/cell.rs`); `GraphicsStore::remove_virtual_placement`, `GraphicsStore::get_placeholder_graphic` (`src/graphics/mod.rs`); `GraphicsStore::export_json_pretty` (`src/graphics/serialization.rs`); `text_utils::get_url_at`, `text_utils::get_line_unwrapped`, `text_utils::find_matching_bracket` (`src/text_utils.rs`) — the live implementations are `Terminal::get_url_at`/`Terminal::find_matching_bracket` (`src/terminal/search.rs`) and `Terminal::get_line_unwrapped` (`src/terminal/screen.rs`). Kept despite audit flags: `Grid::export_visible_screen_styled` (called by `Terminal::export_visible_screen_styled` and the streaming session), `sample_half_block`/`cell_size` (canonical implementations for the bindings delegation), `python/par_term_emu_core_rust/debug.py` (imported by `par-term-emu-tui-rust`), and `MouseMode::X10` (to be wired by ENH-006).

## [0.46.0] - 2026-08-03

### Added
- **`sim` cargo feature: a headless terminal profile** (`Cargo.toml`, `src/lib.rs`, `CLAUDE.md`, `docs/BUILDING.md`). New `sim = []` feature compiles grid + terminal + screenshot only, for pure-Rust embedders that vendor the crate as a server-side screen model (e.g. par-hack's char-mode simulation) without spawning real processes. The real-PTY backend is now gated behind a new `pty_session` feature that makes `portable-pty` and `nix` optional; `python` (the `PyPtyTerminal` binding) and `streaming-bin` (the server binary) auto-enable `pty_session`, so the default build, the PyPI wheel, and `par-term-streamer` are behaviorally unchanged. `graphics`/`sixel` remain compiled under `sim` because the `Terminal` and the screenshot renderer depend on them intrinsically (a headless VT terminal must still parse inline sixel/iTerm2/Kitty graphics). Consume with `cargo build --no-default-features --features sim` — the resulting dep tree contains neither `portable-pty` nor `nix`.

### Fixed
- **Rust debug log now writes to the system temp dir instead of a hardcoded `/tmp`** (`src/debug.rs`). The Unix branch hardcoded `/tmp/par_term_emu_core_rust_debug_rust.log` while only the Windows branch used `std::env::temp_dir()`, so on macOS the Rust log landed in `/tmp` while the Python TUI's log went to the per-user temp dir (`tempfile.gettempdir()`), diverging from the documented `$TEMP` behavior. Both branches now use `std::env::temp_dir().join(...)`; Linux is unchanged (`temp_dir() == /tmp`), and on macOS the Rust log joins the Python log in the per-user temp dir.

### Changed
- **Removed dev-tooling integrations.** The gitnexus and graphify knowledge-graph integrations were removed (sections dropped from `CLAUDE.md`/`AGENTS.md`, config and `.gitignore` entries cleaned up). No library API or runtime behavior changes.

### Dependencies
- **Rust (`cargo update`).** Patch + minor bumps across the dependency tree with no major-version changes; `Cargo.lock` is gitignored (library convention), so this verified compatibility rather than committing a lockfile. Notable: `pyo3` 0.29.0 → 0.29.1, `tokio` 1.52 → 1.53, `read-fonts` 0.39 → 0.41, `skrifa` 0.42 → 0.44, `zerocopy` 0.8.54 → 0.55, plus `serde`/`serde_json` patches. `Cargo.toml` requirements are intentionally loose and unchanged.
- **Python (`uv lock --upgrade`).** Bumped `coverage`, `filelock`, `platformdirs`, `python-discovery`, `virtualenv`, `pre-commit` 4.6.0 → 4.6.1, `ruff` 0.15.21 → 0.16.1, and **`websockets` 16.0 → 17.0.1 (major)** — the streaming client tests pass unchanged against the v17 API. Synced the `>=` floors in `pyproject.toml` `[dependency-groups]` for `pre-commit`, `ruff`, and `websockets`.
- **Web frontend (`web-terminal-frontend/`, `ncu -u` + `bun install`).** Patch/minor bumps: `next` 16.2 → 16.3, `react`/`react-dom` 19.2.7 → 19.2.8, `@bufbuild/protobuf` 2.12 → 2.13, `eslint` 10.6 → 10.8, and **`postcss` 8.5.16 → 8.5.25 (Dependabot advisory fix)**; TypeScript held at 6.0.3 (7.x breaks the `@typescript-eslint`/Next.js toolchain). Also resolved a transitive `brace-expansion` DoS advisory (GHSA-3jxr-9vmj-r5cp / -mh99-v99m-4gvg / -rgw5-rvv9-x895, pulled in via `eslint-config-next`/`typescript-eslint`) by regenerating both lockfiles to the fixed patch versions (`brace-expansion` 1.1.18 / 5.0.9) — `npm audit` now reports 0 vulnerabilities. Removed an empty stray root `package-lock.json` (no `package.json`) that was triggering a Next.js workspace-root warning.
- **Test robustness:** recalibrated the `cloning_combining_cells_is_fast_at_scale` perf guard budget from 200ms → 1000ms (`src/cell.rs`). The `cargo update` codegen drift pushed full-suite clone timing marginally past the old 200ms budget while the guard's target regression (a `SmallVec` → `Vec<char>` revert, a seconds-scale allocation storm) remains caught with wide margin.

## [0.45.0] - 2026-07-09

### Added
- **Host-supplied window state for XTWINOPS reports 11/13** (`src/terminal/mod.rs`, `src/terminal/sequences/csi/window.rs`). The core is headless and previously hardcoded `CSI 11 t` to always report non-iconified and `CSI 13 t` / `CSI 13 ; 2 t` to always report position `(0, 0)`. GUI hosts (e.g. par-term) can now call `Terminal::set_window_iconified(bool)` and `Terminal::set_window_position(x: i32, y: i32)` (with `window_iconified()` / `window_position()` getters) so these reports reflect the real on-screen window state; defaults are unchanged when the host never calls the setters. CSI reply parameters are unsigned, so a negative host-supplied coordinate (possible on multi-monitor setups) is clamped to 0 in the `CSI 13 t` reply. Exposed on both Python `Terminal` and `PtyTerminal` as `set_window_position()`/`window_position()` and `set_window_iconified()`/`window_iconified()`.

### Changed
- **MSRV raised 1.88 → 1.90** (`Cargo.toml`; docs synced in `README.md`, `docs/BUILDING.md`, `docs/CROSS_PLATFORM.md`, `docs/MATURIN_BEST_PRACTICES.md`). No dependency requires it (the tree still builds at 1.88); set to a recent stable for broader compatibility. **Breaking for Rust consumers**: the `rlib` now requires Rust ≥ 1.90; PyPI (Python wheel) users are unaffected (CI builds wheels with the stable toolchain). Rust's Linux glibc floor is unchanged at 2.17 (manylinux2014).

### Fixed
- **Generation counter could lead the grid content, freezing partial regions in TUI apps** (`src/pty_session.rs`). The PTY reader bumps the update-generation counter immediately on a successful read, *before* the grid is written (issue #60 liveness — the counter must advance even if processing panics, e.g. Windows ConPTY after Ctrl+C). But that left a window: a renderer acquiring the terminal lock between the bump and the grid write reads the not-yet-updated grid yet stamps its cell cache with the already-advanced generation. When it was the last read of an output burst, the counter never advanced again, so the stale content was served until the next PTY read — full-screen editors and pagers that repaint via partial line edits (joe, vim: DL/IL/EL) showed "some regions don't update" after scrolling or paging, clearing only on the next keypress/resize. A second generation bump now runs after the grid write (still inside the write guard), guaranteeing the counter always moves past any value a renderer could have observed mid-write, so the next frame regenerates instead of freezing. The pre-processing bump is retained, so issue #60's liveness guarantee is unaffected.

### Dependencies
- **Rust, Python, and web-frontend dependency refresh.** Rust `cargo update` brought 5 minor + 39 patch bumps with no major-version changes; Python (`uv lock --upgrade`) bumped pillow, maturin, pyright, pytest, and ruff (and synced the `>=` floors in `pyproject.toml`); the web frontend (`ncu`) took patch bumps for next/buf/tailwind/postcss plus **pako 2 → 3**, whose default-export removal required a one-line import fix in `lib/protocol.ts` (the now-redundant `@types/pako` was dropped, since pako 3 ships its own types). TypeScript is held at 6.0.3 (7.0 breaks the `@typescript-eslint` toolchain and the Next.js build worker). The committed `web_term/` static output was rebuilt.

## [0.44.0] - 2026-07-03

### Security
- **Kitty PNG + iTerm2 image-decode size caps (decompression-bomb DoS).** Kitty PNG graphics were decoded via `image::load_from_memory` with no dimension bound (unlike the sibling raw `Rgba`/`Rgb` branches and the iTerm2 path), so a small, highly-compressible PNG could decode to tens of GiB of RGBA and OOM the host — reachable from any bytes written to the terminal. Both `src/graphics/kitty.rs` (`decode_pixels`) and `src/graphics/iterm.rs` (`decode_image`) now decode through a size-limited `image::ImageReader` and enforce a shared `MAX_IMAGE_PIXELS` product cap (new constants in `src/graphics/mod.rs`); iTerm2 previously bounded each axis individually but not the product (a ~1 GiB single image previously passed).
- **CLI secrets no longer printed by `--help`.** Added `hide_env_values` to the `api_key`, `http_password`, and `http_password_hash` streaming-server CLI args, so `--help` no longer echoes the live secret when the backing environment variable is set.

### Added
- **OSC 99 Kitty desktop notifications** (`src/terminal/sequences/osc/notify.rs`, `handle_osc99`). Format `OSC 99 ; <metadata> ; <payload> ST`, where `<metadata>` is zero or more colon-separated `key=value` pairs: `i` (id, groups/updates chunks), `d` (done: `0` = more chunks, `1` = last, default `1`), `p` (payload type: `title` default or `body`), `e` (`0` raw default, `1` base64), `u` (urgency: `0` low, `1` normal default, `2` critical), `a` (comma-separated actions, e.g. `focus,report,close`). Unknown keys are ignored for forward compatibility; multi-chunk payloads sharing an `i=` id accumulate until `d=1`. Complements the existing OSC 9/777 notifications.
- **XTGETTCAP** (`DCS + q`, `src/terminal/sequences/dcs/query.rs`). Replies per requested hex-encoded capability name: `DCS 1 + r <hexname>=<hexvalue> ST` if known, `DCS 0 + r <hexname> ST` if unknown. Supports `TN`/`name` (`xterm-256color`), `Co`/`colors` (`256`), `RGB` (`8`, bits per channel), and `Tc` (truecolor flag).
- **DECRQSS** (`DCS $ q`, same file). Replies `DCS 1 $ r <current-setting><final> ST` for a recognized mnemonic or `DCS 0 $ r ST` otherwise. Supports `m` (SGR attributes), ` q` (DECSCUSR cursor style), and `r` (DECSTBM scroll margins).
- **XTWINOPS report ops 11/13/19** (`src/terminal/sequences/csi/window.rs`). `CSI 11 t` reports window state (always non-iconified — a headless/library core has no window), `CSI 13 t` / `CSI 13 ; 2 t` reports window position (`CSI 3 ; 0 ; 0 t`), and `CSI 19 t` reports screen size in characters (`CSI 9 ; rows ; cols t`). The window-manipulation ops (1/2/3/4/5/6/9/10 — deiconify/iconify/move/resize/raise/lower/maximize/fullscreen) and `Ps >= 24` resize remain explicit no-ops for the same reason.
- **Python `take_notifications_detailed()` + `Notification` class.** New method on both `Terminal` and `PtyTerminal` returns `Notification` objects exposing the OSC 99 `id`, `urgency` (`"low"`/`"normal"`/`"critical"`), and `actions` metadata. `take_notifications()`/`drain_notifications()` are unchanged and still return `(title, message)` tuples — this is a non-breaking addition.

### Fixed
- **`DCS +q`/`DCS $q` were misrouted to the Sixel parser.** DCS routing keyed only on the final `q` byte, so XTGETTCAP (`+q`) and DECRQSS (`$q`) queries were dispatched into the Sixel graphics handler instead of their own handlers. Routing now also considers the intermediate byte (`+` vs `$` vs none), sending each to the correct handler.

### Performance
- **Observer/trigger dispatch and PTY device-query replies moved outside the terminal write lock.** The PTY reader thread previously held `Terminal`'s exclusive write guard for the whole `process()` call, including observer/trigger callback dispatch (which can re-enter Python under the GIL) and the blocking write of device-query responses back to the PTY master — both stalled every concurrent reader (streaming clients, Python queries, screenshot rendering), nullifying the earlier `Mutex` → `RwLock` migration. `process_deferred()` now returns an owned `ObserverDispatchBatch` delivered after the write guard drops (preserving event ordering and existing panic isolation), and device-query response bytes are drained inside the guard but written to the PTY only after it is released.
- **Screenshot glyph bitmaps shared via `Arc<[u8]>` instead of deep-copied.** `CachedGlyph.bitmap` was a `Vec<u8>` cloned on every rendered character to release the font-cache borrow, so an 80x24 screenshot triggered ~1,920 heap copies per call; it's now `Arc<[u8]>`, so the clone is a refcount bump.
- **Removed per-row `String` allocation in screenshot flag-emoji detection.** `render_grid` built and discarded a full-row `String` every frame just to detect regional-indicator flag emoji; it now calls `row_has_regional_indicators`, which scans grid cells directly with no allocation.

### Documentation
- **Fixed ARCHITECTURE.md/API_REFERENCE.md drift from the 0.43.0 refactor.** Docs still showed `Arc<Mutex<Terminal>>` (now `RwLock`), a flat `Terminal` struct (now ~30 sub-structs), and wrong signatures for `regex_search`/`search_scrollback`/`record_mouse_event` plus incorrect mouse enum values; a full scan of all pyo3 signature blocks fixed those and other drift (e.g. `export_scrollback` documented values that actually raise `ValueError`). Documented the `ScreenshotConfig` and `StreamingConfig.allowed_origins` APIs, and backfilled Google-style Args/Returns/Example docstrings across the color, mouse, and clipboard binding modules.

### Changed
- **`python_bindings/types.rs` god-file split (ARC-M1/QA-M5, internal — no API change).** The 4,004-line catch-all `types.rs` (56 `#[pyclass]` data types in one module) was split by domain into a `types/` directory of 12 cohesive submodules (`screen`, `graphics`, `shell`, `clipboard`, `notification`, `selection`, `metrics`, `color`, `mouse`, `session`, `trigger`, `recording`) behind a thin `types/mod.rs` facade that `pub use`-re-exports every type — so `python_bindings::types::PyX` and the crate-level re-exports resolve exactly as before (no change to `lib.rs` or `python_bindings/mod.rs`). The 25 unit tests moved with the types they exercise (regaining module-private method access). Pure line-move — no type, field, method, or behavior change; the full Rust + 495 Python tests pass.
- **Deduplicated WS/WSS handshake header-callback logic.** The plain and TLS WebSocket listeners each duplicated ~100 lines of handshake logic (origin validation, auth validation, query capture), so a fix to the origin check could silently miss one transport; both now share a single `build_ws_header_callback` factory. Also gated the web frontend's debug `console.log` calls behind a `NODE_ENV`-checked helper (`console.error`/`console.warn` remain real diagnostics).

## [0.43.1] - 2026-06-17

### Security
- **Kitty graphics protocol: integer-overflow DoS in `decode_pixels`** (`src/graphics/kitty.rs`). Raw RGBA/RGB size checks and the RGB `Vec::with_capacity` used `width * height * (4|3)` with attacker-controlled `u32` dimensions that could wrap `usize`, bypass the size check, and yield a graphic with huge dimensions over a tiny buffer → out-of-bounds read/panic on malicious terminal/SSH graphics output. Now uses `checked_mul` and rejects overflowing dimensions. Regression tests added.
- **Web-frontend dependency advisories resolved (`npm audit`: 4 → 0).** `@babel/core` (CVE-2026-49356, arbitrary file read via `sourceMappingURL`), `postcss` (CVE-2026-41305, XSS via unescaped `</style>`) nested under `next`, and `brace-expansion` (GHSA-jxxr-4gwj-5jf2). Fixed via `npm audit fix` plus an npm `overrides` entry that dedupes the nested `postcss` to the safe top-level 8.5.15 — deliberately avoiding npm's suggested `--force`, which would have downgraded Next.js 16 → 9. Build-time-only; the shipped `web_term/` output is unchanged.

### Fixed
- **Kitty `V=` relative offset now parsed unconditionally.** `parse_chunk` dropped `relative_y_offset` (`V=`) unless `P=` (parent) had already been parsed, while `H=` (`relative_x_offset`) was unconditional. The offset is only applied when a parent exists, so the guard only caused `V=` to be silently dropped; it now matches `H=`.
- **Kitty `parse_delete_target` handles `i`/`p`/`x`/`y`.** The parser only emitted `All`/`AtCursor`/`OnScreen` (`a`/`c`/`z`), so `d=i`/`d=p`/`d=x`/`d=y` were silently ignored and the matching `build_graphic` Delete arms (`ById`/`ByPlacement`/`ByColumn`/`ByRow`) were dead code. They are now parsed (the identifying `i=`/`x=`/`y=` param must precede `d=`, else the delete is a safe no-op). Regression tests added.

## [0.43.0] - 2026-06-17

### ⚠️ Breaking — Rust API only

> Affects **direct Rust consumers of the library only** (e.g. `par-term`). The **Python bindings and runtime behavior are unchanged** — the `Terminal`/`PtyTerminal` Python API surface is identical, and all 492 Python tests plus the full Rust suite pass. These two items shipped in 0.43.0 but were originally listed under "Performance"/"Changed" as non-breaking; they are in fact breaking for the Rust crate and are reclassified here for accuracy. Python-only consumers can ignore this section.

- **ARC-009 — `Arc<Mutex<Terminal>>` → `Arc<RwLock<Terminal>>` (breaking type change).** The `PtySession.terminal` field and `PtySession::terminal()` now expose `Arc<RwLock<Terminal>>` instead of `Arc<Mutex<Terminal>>`. Runtime semantics are preserved (mutators still serialize exactly as under the old `Mutex`), but the lock **type** changed, so any Rust site doing `term.terminal().lock()` no longer compiles — `RwLock` exposes `.read()` / `.write()`, not `.lock()`. **Migration:** replace each `.lock()` with `.read()` for read-only queries (cursor position, content, mode/state getters) and `.write()` for mutators (`process`, `write`, `resize`, `reset`). The locks are `parking_lot`, so no `.unwrap()` is needed — that read-vs-mutator distinction is the classification rule for converting every `.lock()` site.
- **ARC-012 — `Cell` / `Grid` fields are now encapsulated (breaking field-access change).** `Cell`'s fields are `pub(crate)` (`c`, `fg`, `bg`, `flags`, `combining`, `underline_color`, `width`) and `Grid`'s fields are `pub(in crate::grid)`, so direct field access from outside the crate no longer compiles — e.g. `cell.fg`, `cell.bg`, `cell.c`, `cell.flags`, `grid.cells`. **Migration:** use the accessor methods — `Cell::c()`, `Cell::fg()`, `Cell::bg()`, `Cell::flags()` (returns `&CellFlags`), and `Grid::cols()` / `rows()` / `get(col, row)` / `get_mut(..)` / `row(row)` / `row_mut(row)`. This mirrors what the Python `#[pyo3(get)]` surface already exposed, so only direct Rust field access is affected.

### Security
- **Streaming server: unbounded zlib decompression (zip-bomb DoS).** `decode_with_decompression` in `src/streaming/proto.rs` called `ZlibDecoder::read_to_end` with no size cap, so a single small compressed protobuf frame could expand into gigabytes of memory and OOM-kill the server. It now reads from the decoder in 8 KiB chunks into a bounded `Vec` and returns `StreamingError::InvalidMessage` at the new 1 MiB `MAX_DECOMPRESSED_SIZE` cap. Additionally, both tungstenite WebSocket acceptors now use `accept_hdr_async_with_config` with an explicit `WebSocketConfig` (16 MiB `max_message_size` / `max_frame_size`) instead of relying on tungstenite's 64 MiB default, and the same 16 MiB caps are applied to both axum `WebSocketUpgrade` handlers. No new dependencies; return types and error variants unchanged.
- **Dependency vulnerabilities resolved (`cargo audit`: 5 → 0 vulnerabilities).** Replaced the abandoned `htpasswd-verify` crate (which pulled the unmaintained `rust-crypto` RUSTSEC-2022-0011, `rustc-serialize` RUSTSEC-2022-0004, `time` RUSTSEC-2020-0071, and `gcc` RUSTSEC-2025-0121) with direct htpasswd-hash verification in a new `src/streaming/auth_hash.rs` module using maintained RustCrypto primitives (`bcrypt`, `md-5`, `sha1`) + the existing `base64` dep — supports the same `$2y$`/`$apr1$`/`$1$`/`{SHA}` formats, with the MD5-crypt core ported from the canonical crypt(3) algorithm and locked down with `openssl`-generated known-answer vectors. Upgraded PyO3 0.28.3 → 0.29 (RUSTSEC-2026-0176/0177). Replaced the unmaintained `rustls-pemfile` (RUSTSEC-2025-0134) with the `rustls-pki-types` `PemObject` trait (already transitively available — no new dependency). Disabled the AVIF feature on the `image` crate so the unmaintained `paste` proc-macro (RUSTSEC-2024-0436, transitive via `ravif` → `rav1e`) is no longer compiled into any build.
- **Streaming server: CSRF-via-WebSocket defense.** New optional `StreamingConfig.allowed_origins` allowlist (also exposed as the Python `allowed_origins` property and the standalone `--allowed-origins` CLI flag / `PAR_TERM_ALLOWED_ORIGINS` env var). The WebSocket `Origin` header is now validated at all four WS entry points (two tungstenite acceptors + two axum handlers): by default, non-browser clients (no `Origin` header — curl, native TUI, the embedded library) and local (`localhost`/`127.0.0.1`/`::1`) browser origins are accepted while remote browser origins (e.g. a malicious page on `evil.com`) are rejected with HTTP 403; when an allowlist is configured, only those exact origins are accepted. A `tower-http` `CorsLayer` reflecting the same policy is applied to both HTTP routers.
- **Standalone streamer: warn on public bind without auth.** The binary already defaults `--host` to `127.0.0.1`; it now also prints a loud stderr warning when binding a non-loopback interface with no `--api-key` or HTTP Basic auth configured, since that exposes an interactive shell to anyone who can reach the port.

### Changed
- **Internal code-quality refactors (no API or behavior change).** From the project audit: removed the `Ok::<_, ()>(lock())` dead-branch anti-pattern at 12 sites in `src/streaming/server.rs` (`parking_lot::Mutex::lock` cannot fail); rewrote `html_escape`/`escape_xml` to allocate a single `String` instead of one per character; made `get_dirty_region` a single pass with no panic-bait `unwrap()`; `poll_subscribed_events` now calls the existing `TerminalEvent::kind()` instead of duplicating its 25-arm match; the coprocess output/error buffers switched `Vec<String>` → `VecDeque<String>` (O(1) `push_back`/`pop_front` instead of O(n) `remove(0)`); and the duplicated `emit_style` SGR closure (~78 lines × 2) in `src/grid/export.rs` was extracted into a shared `push_sgr_style` helper.
- **`Terminal` god-object decomposition (ARC-001, internal — no API or behavior change).** The `Terminal` struct's ~150 flat fields were grouped into 31 cohesive `pub(crate)` sub-structs (e.g. `TerminalModes`, `ColorThemeState`, `ClipboardSyncState`, `ProfilingState`, `TriggerState`, `DcsState`, `MarginState`, `EventBrokerState`, `GraphicsState`), each held as a single field on `Terminal`. The remaining ~25 flat fields are the irreducible core (buffer, cursor, parser, current cell render state). Existing accessor methods on `Terminal` delegate to the sub-structs, so all callers — including the Python bindings — are unaffected. Behavior is preserved (the full Rust + Python test suites pass).
- **`PyTerminal` Python-binding split (ARC-002, internal — no Python API change).** The single ~5768-line / ~384-method `#[pymethods]` block on the `Terminal` Python class was split into 18 cohesive blocks: the core methods remain in `python_bindings/terminal/mod.rs` (halved to ~2869 lines) and 17 themed sibling files (`trigger_api`, `color_api`, `clipboard_api`, `metrics_api`, `search_api`, `selection_api`, `snapshot_api`, `scrollback_api`, `bookmark_api`, `multiplexing_api`, `image_api`, `shell_integration_api`, `notification_api`, `recording_api`, `badge_api`, `file_transfer_api`, `mouse_api`, `text_api`), each its own `#[pymethods] impl PyTerminal`. Enabled the `pyo3` `multiple-pymethods` Cargo feature so PyO3 merges all blocks into the same Python `Terminal` class — **the Python API surface is unchanged** (492 Python tests pass). (The audit's stretch remedy — nested sub-objects like `term.colors.x` for autocomplete discoverability — is a breaking major-version API change and is not included.)
- **`PyTerminal`/`PtyTerminal` method duplication eliminated (ARC-003/QA-001, internal — no Python API change).** The ~155 methods duplicated between the `Terminal` and `PtyTerminal` Python classes are reduced to **26** via a shared `TerminalAccess` trait (`term_ref`/`term_mut`, RPITIT) + `#[macro_export]` method macros in `python_bindings/common.rs` that define each shared method once and emit it for both classes. **129 methods unified, ~258 duplicate copies deleted, net −1,200+ lines.** The 8 hand-written `PyAttributes` literals are replaced by the existing `From<&Cell>` impl. The remaining 26 duplicates are genuinely non-unifiable (e.g. `PtyTerminal.paste()` writes to the PTY child while `Terminal.paste()` parses locally; `screenshot`/`resize`/`create_snapshot` have divergent logic) — unifying them would change behavior, so they're intentionally kept. Behavior is preserved (492 Python tests pass on every batch).
- **`Cell` combining marks stored inline (ARC-005/QA-004, performance — no API change).** The `Cell` grid's combining-marks field changed from `Vec<char>` to `SmallVec<[char; 4]>`, so cells with ≤4 combining marks (variation selectors, ZWJ, skin-tone modifiers, regional-indicator pairs — the >99.9% case) no longer heap-allocate, and cloning them during scroll/reflow/snapshot is a memcpy instead of a heap allocation. Rare longer clusters spill to the heap (no data loss). `Cell` stays `Clone` (SmallVec is not `Copy`), but per-cell allocation pressure on the grid hot paths is eliminated. A regression benchmark (`cloning_combining_cells_is_fast_at_scale`) locks in the characteristic.
- **WebSocket handler dedup (ARC-004/QA-002, internal — no protocol change).** The 3 near-identical ~380-line WebSocket connection handlers (plain TCP, TLS, axum) in `streaming/server.rs` were collapsed: the `Client` type is now generic over the tungstenite stream type (`Client<S>`), so the plain and TLS handlers share one `run_ws_session` dispatch loop (~770 lines of duplicated 11-arm `ClientMessage` dispatch eliminated — adding a new message type no longer requires editing 3 handlers). The axum handler's pure `RequestRefresh`/`SnapshotRequest` arms now call shared builder helpers. **Bonus parity fix**: the old TLS handler silently dropped `Mouse`/`FocusChange`/`Paste`/`SelectionRequest`/`ClipboardRequest` (a documented inconsistency); the shared loop now handles them uniformly. A new end-to-end WS smoke test (`tests/test_ws_smoke.rs`) guards the refactored path (real server + client, handshake + Ping→Pong round-trip). 174 streaming tests + 492 Python tests pass.

### Fixed
- **Trigger actions now apply to terminal state.** The `Notify`, `MarkLine`, and `StopPropagation` trigger actions were queued as `ActionResult`s but never applied to terminal state (unlike `SetVariable`/`Highlight`, which mutate state immediately), so `notifications()`/`get_bookmarks()` never reflected trigger matches and three trigger integration tests had been failing. `execute_trigger_actions` now applies `Notify` → `enqueue_notification` and `MarkLine` → `add_bookmark` immediately, while still pushing the `ActionResult` for host-side polling — the Python `poll_action_results()` API is unchanged.
- **Panicking terminal observers no longer corrupt state (ARC-007 safe fix).** Observer callbacks (`on_zone_event`/`on_command_event`/`on_environment_event`/`on_screen_event`/`on_event`) now run inside `std::panic::catch_unwind`. Previously, a panicking observer (e.g. a misbehaving Python callback) would unwind through the `Terminal`'s `parking_lot::Mutex` — which does not poison — silently leaving the terminal inconsistent and re-firing already-dispatched events. The panic is now contained to the failing observer; dispatch completes normally and a regression test (`test_panicking_observer_is_isolated`) locks in the behavior. (The separate, larger work of moving observer dispatch fully out of the mutex remains future work.)

### Documentation
- **Default streaming-server port corrected (8080 → 8099)** in README, Makefile, and the `streaming_server` module rustdoc to match the actual clap `default_value`. The README's Rust toolchain requirement was updated 1.75+ → 1.88+ to match `Cargo.toml`'s `rust-version`. The STREAMING.md architecture diagram label now reads "Protobuf (binary)" instead of "JSON Messages". ARCHITECTURE.md gained the missing `apc_filter.rs` and `badge.rs` modules and now points readers at runnable test-count commands instead of stale hard-coded numbers. BUILDING.md gained a prominent "never `cargo build`; use `make dev`" callout. A new `CONTRIBUTING.md` documents the dev workflow, the version-sync and Rust↔Python binding-sync rules, and the PR process. `src/pty_session.rs` gained a module-level rustdoc comment.
- **API_REFERENCE.md accuracy pass (DOC-001..011, DOC-018).** Fixed 12+ Data Classes with wrong property lists (LineDiff, SnapshotDiff, PerformanceMetrics, ProfilingData, RenderingHint, FrameTiming, MouseEvent, RegexMatch, DetectedItem, PaneState, SessionState, EscapeSequenceProfile, TmuxNotification). Corrected `spawn_shell` and `set_selection` signatures, ScreenSnapshot properties, `search()` case-default documentation, missing `Attributes` fields, and type annotations (ColorHSL/HSV float, cursor_color/default_fg/default_bg no longer `| None`). Added 3 missing enums (UnicodeVersion, AmbiguousWidth, WidthConfig). Added 15 missing `PAR_TERM_*` env vars to STREAMING.md. Added a streaming/TLS/auth section to SECURITY.md covering the network threat model, authentication, transport security, Origin validation, and input safety. Fixed stale Rust library version pins (`"0.10"` → `"0.42"`) in README.

### Added
- **Test-coverage tooling and a wider CI gate.** New `make coverage` / `coverage-html` (cargo-llvm-cov) and `coverage-python` (pytest-cov) targets, with `pytest-cov` added to the dev dependencies. `make test-rust` / `test-rust-streaming` no longer pass `--lib`, so the `tests/*.rs` integration tests and the doc-tests now run in the standard gate (the `--lib` skip is what kept the trigger bug above hidden). Overall Rust line coverage rose from ~77% to ~81%, with the previously-uncovered `image` / `compliance` / `macros` modules at 85–100% and the graphics / Kitty parsers at 91–98%. GitHub Actions were bumped to their latest majors and the floating `Ilshidur/action-discord@master` ref was pinned to release `0.4.0`.
- **`ScreenshotConfig` options object (QA-005).** New `ScreenshotConfig` pyclass + `screenshot_config(config)` / `screenshot_to_file_config(path, config)` methods on both `Terminal` and `PtyTerminal`, so callers can build a reusable config instead of repeating 16+ keyword args.
- **`TerminalAction` enum (ARC-021).** A structured, owned mirror of the `vte::Perform` callbacks, enabling test-in-isolation (construct an action and apply it without byte parsing), record→replay, and queueing. Includes `apply_action`, `apply_actions`, `parse_to_actions`, and `to_bytes`.
- **`par-term-emu-derive` proc-macro crate (ARC-014).** New `#[pyo3_get_all]` attribute macro that adds `#[pyo3(get)]` to every named field of a `#[pyclass]` data struct — eliminates 262 hand-written `#[pyo3(get)]` attribute lines across 47 data classes.
- **Configurable OSC data limit (QA-012).** `Terminal::max_osc_data_length()` / `set_max_osc_data_length()` — the 128 MiB OSC data cap is now runtime-configurable for tighter memory/security bounds.
- **Makefile `typecheck` + `clippy` targets (ARC-024).** Standalone `make typecheck` (cargo check --all-targets --all-features + pyright) and `make clippy` (check-only, no auto-fix).
- **Proto staleness build check (ARC-020).** `build.rs` now warns when `proto/terminal.proto` is newer than the checked-in `terminal.pb.rs`.

### Performance
- **`Terminal` read concurrency (ARC-009 — ⚠️ breaking Rust type change; see "⚠️ Breaking — Rust API only" above).** `Arc<Mutex<Terminal>>` → `Arc<RwLock<Terminal>>` — the ~129 Python read-query methods now take a shared read lock, so concurrent reads don't spin-wait on the reader thread's `process()` call. Mutators and the reader thread take write locks. Runtime behavior is identical to the old Mutex, but the lock **type changed**, so Rust callers must move `.lock()` → `.read()`/`.write()` (Python is unaffected).
- **`process()` APC passthrough buffer reuse (ARC-008).** The Kitty APC pre-filter's passthrough buffer is now a reusable field (`mem::take`/restore) instead of a fresh `Vec` per call. ESC-free chunks skip the filter entirely when the APC state machine is idle.
- **`row_text`/`cells_to_text` single-String allocation (QA-006).** Added `Cell::push_grapheme(&mut String)`; both functions now write into one pre-sized `String` instead of building a `Vec<String>` per row.
- **Glyph cache bounded LRU (ARC-017).** Replaced the nuclear `HashMap::clear()` at 10k entries with a bounded `lru::LruCache` — least-recently-used glyphs evict one at a time instead of clearing the whole cache.
- **`Cell.hyperlink_id` niche optimization (ARC-010).** `Option<u32>` → `Option<NonZeroU32>` — `None` (the common no-link case) costs zero extra bytes, saving ~4 bytes per cell across millions of scrollback cells.

### Changed
- **Architecture refactors (internal — no API change).** `terminal_events` queue capped at 10k with oldest-eviction + DRY typed polls (ARC-006). `TerminalEvent → ServerMessage` dispatch consolidated into one shared function + proto-staleness build check (ARC-020). Screenshot iTerm2 P3 color boost gated behind `ScreenshotConfig.iterm2_color_boost` (ARC-022). Scrollback circular-buffer index math centralized into helpers (ARC-026). Confusingly-named snapshot modules renamed (`terminal_snapshot`→`replay_snapshot`, `snapshot`→`semantic_snapshot`) (ARC-027). `streaming` feature split: binary-only deps (clap, reqwest, tar, etc.) moved to a new `streaming-bin` feature so library embedders shed them (ARC-015). `Cell`/`Grid` fields encapsulated behind accessor methods (ARC-012 — ⚠️ breaking Rust field-access change; see "⚠️ Breaking — Rust API only" above). `bin/streaming_server.rs` split into a module directory with `theme.rs` extracted (QA-013).
- **Code-quality improvements.** `LineCellData` alias applied — 2 `#[allow(clippy::type_complexity)]` suppressions removed (QA-011). `From<DomainError> for PyErr` centralized for all 4 domain error enums (QA-009). `.clone()` audit: all clones necessary (borrowed/non-Copy sources) — no beneficial moves identified (QA-008). pyright `pythonVersion` aligned to 3.12 floor (ARC-023).
- **Observer reentrancy guard (ARC-016).** A `thread_local!` flag prevents Python observer callbacks from re-entering terminal processing on the same thread, which would deadlock on the non-reentrant Terminal mutex.
- **PTY `Drop` force-closes master fd (ARC-018).** `Drop for PtySession` now explicitly drops the PTY master *before* the reader-thread join-wait (mirroring the restart path), so the blocked `read()` returns EOF and the thread joins instead of being silently detached at the 2-second timeout.

## [0.42.4] - 2026-06-08

### Fixed
- **`has_updates_since()` unreliable on Windows after Ctrl+C.** The generation counter (`update_generation` / `has_updates_since`) was incremented at the end of a long terminal processing block in the PTY reader thread. If processing encountered a panic or unexpected code path — notably on Windows ConPTY after the child receives `CTRL_C_EVENT` — the counter would stay stuck even though data had already been written to the grid buffer. The `fetch_add(1)` now runs immediately after a successful `reader.read()`, before any processing, guaranteeing the counter always advances when PTY data arrives. ([#60](https://github.com/paulrobello/par-term-emu-core-rust/issues/60), [#61](https://github.com/paulrobello/par-term-emu-core-rust/pull/61))

## [0.42.3] - 2026-06-06

### Fixed
- **BCE (Background Color Erase) not implemented.** All VT erase operations (`EL` `\x1b[K`, `ED` `\x1b[J`, `ECH` `\x1b[X`, `DECSERA`) now fill erased cells with the current SGR background color instead of always resetting to black. This matches the behavior of iTerm2, Alacritty, and Windows Terminal. Grid erase methods now accept a `bg: Color` parameter; the CSI erase handler passes `self.bg` (current SGR background). Scroll/edit operations continue using default black bg (BCE does not apply to scroll). ([#57](https://github.com/paulrobello/par-term-emu-core-rust/issues/57), [#58](https://github.com/paulrobello/par-term-emu-core-rust/pull/58))

### Build / Tooling
- **Updated Rust crates:** bitflags 2.11.1→2.13.0, uuid 1.23.1→1.23.2, tokio 1.52→1.52.3, reqwest 0.13.3→0.13.4, sysinfo 0.39.2→0.39.3, unicode-segmentation 1.13→1.13.3, plus transitive bumps (hyper, log, zerocopy, yoke, etc.)
- **Updated Python dev deps:** pyright 1.1.409→1.1.410, pytest-asyncio 0.26→1.4, ruff 0.15.14→0.15.16
- **Updated web frontend deps:** next 16.2.6→16.2.7, react 19.2.6→19.2.7, react-dom 19.2.6→19.2.7, eslint 10.4.0→10.4.1, eslint-config-next 16.2.6→16.2.7, @types/node 25.9.1→25.9.2, @types/react 19.2.15→19.2.17

## [0.42.2] - 2026-05-29

### Fixed
- **PytestUnknownMarkWarning for `@pytest.mark.asyncio`.** Added `pytest-asyncio` to dev dependencies so pytest recognizes the asyncio mark used by streaming tests.
- **Top-anchored scroll region lines lost instead of entering scrollback.** `scroll_region_up()` only pushed rows into scrollback when the region covered the full screen. Partial top-anchored regions (e.g. Codex CLI's `CSI 1;{rows-1} r`) now correctly preserve evicted rows in primary-screen scrollback, matching iTerm2 behavior. Refactored scrollback insertion into a shared `push_rows_to_scrollback()` helper used by both full-screen and region scrolling paths. ([#54](https://github.com/paulrobello/par-term-emu-core-rust/pull/54), thanks @ziglerari)

### Build / Tooling
- **Updated Rust crates:** tikv-jemallocator 0.6→0.7, sysinfo 0.39.2→0.39.3, hyper 1.9→1.10, reqwest 0.13.3→0.13.4, plus transitive bumps (cc, displaydoc, http, log, memchr, mio, socket2, uuid, zerocopy)
- **Updated Python dev deps:** pygments 2.19.2→2.20.0, platformdirs 4.9.6→4.10.0, pytest-asyncio 1.3→1.4, ruff 0.15.14→0.15.15, virtualenv 21.3→21.4
- **Updated web frontend deps:** @bufbuild/buf 1.69→1.70

## [0.42.1] - 2026-05-08

### Fixed
- **Kitty TGP virtual placement: image truncated past 64th column/row.** `graphics::placeholder::diacritic_to_number` only mapped the first 64 of the 297 diacritics defined in the Kitty graphics protocol spec ([rowcolumn-diacritics.txt](https://github.com/kovidgoyal/kitty/blob/master/kittens/unicode_input/rowcolumn-diacritics.txt)). Once a client emitted a placeholder cell with `col >= 64`, the second-diacritic lookup returned `None`, `decode_placeholder_cell` rejected the cell, and the bounding-box scan in par-term-render's `scan_placeholder_cells` stopped at column 63 — so a Kitty image asked to fit in e.g. 75 cells rendered into only 64. Replaced the 64-arm match with the full 297-entry `DIACRITICS: &[char]` static table plus a `OnceLock<HashMap<char, u16>>` reverse lookup. `row` and `col` widened from `u8` to `u16` (the spec's 0..=296 range overflows `u8`); `msb` stays `u8` because that diacritic position only encodes 0..=255. New tests `test_diacritic_table_size` (asserts 297 entries) and `test_create_placeholder_past_64` (round-trips index 120/200) lock down the regression.
- **CSI 16t (report cell pixel size) returned a hardcoded 10×20.** `XTWINOPS Ps=16` now derives the response from `self.pixel_width / cols` and `self.pixel_height / rows` (both set by `Terminal::set_pixel_size` from host-supplied resize events), with the 10×20 default kept as a fallback for the pre-resize state. Clients querying `\x1b[16t` to size inline graphics (sixel/iTerm2/Kitty) now receive the renderer's actual cell pitch instead of a value that disagreed with both the renderer and `TIOCGWINSZ`.

### Build / Tooling
- **GitHub Actions updated to latest major versions.** Bumped `actions/checkout`, `actions/setup-python`, `actions/upload-artifact`, and `actions/download-artifact` to their current major versions.
- **Streaming binary build timeout increased to 30 minutes.** The cross-compilation step for the streaming server binary was hitting the default 20-minute timeout on slower runners.

## [0.42.0] - 2026-05-06

### Added
- **Kitty Terminal Graphics Protocol — query response (Phase 3 of 3)**: `KittyParser` now stores the `q=` (quietness) parameter as `pub quietness: u8`. `Terminal::filter_apc_and_advance` branches on `KittyAction::Query`: when `quietness < 2`, builds an APC reply (`ESC _ G [i={id};] OK ESC \`) and appends it to `response_buffer` (the same buffer ENQ/DA1/DSR replies use). `q=2` suppresses the reply. Echoes the inbound `i=<id>` if present; emits `\x1b_G;OK\x1b\\` if not. Four new tests (`query_emits_ok_response_on_response_buffer`, `quiet_mode_query_suppresses_response`, `query_without_image_id_emits_ok_without_id`, `transmit_does_not_emit_response`).
- **Kitty Terminal Graphics Protocol — APC ingestion (Phase 1 of 3)** _(Phase 2 lives in par-term/par-term-render — see par-term CHANGELOG)_: `Terminal::process` now intercepts Kitty TGP APC sequences (`ESC _ G ... ST`) before they reach `vte::Parser`. The previously orphaned `KittyParser` (in `src/graphics/kitty.rs`) is now wired into the byte stream:
  - New module `src/terminal/apc_filter.rs` implements a streaming byte-level state-machine pre-filter. Only APCs starting with `_G` (Kitty graphics) are intercepted; all others (`_X…`, etc.) pass through unchanged so that `vte` continues to silently swallow them per `State::SosPmApcString` semantics.
  - The pre-filter is necessary because `vte` 0.15 does not deliver APC payload bytes to `Perform` — `SosPmApcString` only handles state transitions, never invoking a payload callback.
  - Handles APC sequences split across multiple `process()` calls (the common case — Kitty TGP messages typically arrive as 4 KB chunks, each in its own APC), both `\x1b\\` (7-bit ST) and `\x9c` (C1 ST) terminators, and `ESC` bytes within payloads.
  - Completed Kitty APC payloads are forwarded to `KittyParser::parse_chunk`; on the final chunk (no `m=1`), `build_graphic` commits transmitted images and virtual placements (`U=1`) into `GraphicsStore`. Malformed APCs reset the parser and are silently dropped (no panic).
  - Three new fields on `Terminal`: `apc_filter_state`, `apc_buffer`, `kitty_parser`.
  - 10 unit tests for the filter state machine + 5 integration tests against `Terminal::process` covering split sequences, surrounding-text passthrough, and non-Kitty APC ignore.
- Phase 2 (rendering placeholder cells via virtual-placement lookup, in par-term-render) and Phase 3 (responding `OK` to TGP query commands so terminal-detection probes succeed) are still required for end-to-end visible Kitty TGP support.

### Fixed
- **Kitty TGP placeholder rendering: par-term hang on first display.** Writing an N×M Kitty TGP placeholder rectangle (e.g. 40×20 = 800 cells × 4 chars per cell = 3200 prints + 2400 combining-mark insertions) ran the default NFC Unicode normalization on every char and every combining-mark insertion, allocating thousands of intermediate `String`s and recalculating grapheme widths inside `Terminal::process` while the terminal write lock was held. par-term's renderer (which `try_read`s the same lock per pane per frame) saw the lock contested for hundreds of milliseconds and skipped frames; from the user's perspective every tab froze. Added a fast-path bypass: `print` skips normalization for `\u{10EEEE}` (the Kitty placeholder char) and `write_char`'s combining-mark branch skips normalization + width recalc when the target cell's base char is `\u{10EEEE}`. These cells encode an image ID, not real text, so the work was pure waste. New benchmark test `placeholder_cells_render_at_scale_quickly` asserts 40×20 placeholder ingestion completes in < 200 ms (in practice well under 10 ms). Plus `placeholder_cells_skip_unicode_normalization` pins down the cell-shape invariant.

### Changed
- **Dependency updates across all three sub-projects**:
  - **Rust** (`Cargo.toml`): bumped tokio 1.51→1.52, axum 0.8.8→0.8.9, tower-http 0.6.8→0.6.10, rustls 0.23.37→0.23.40, reqwest 0.13.2→0.13.3, bitflags 2.11.0→2.11.1, clap 4.6.0→4.6.1, uuid 1.23→1.23.1, libc 0.2.184→0.2.186.
  - **Python** (`pyproject.toml`): bumped rich 14→15, ruff 0.15.10→0.15.12, pyright 1.1.408→1.1.409, pre-commit 4.5.1→4.6.0.
  - **Frontend** (`web-terminal-frontend/package.json`): bumped next 16.2.3→16.2.5, react/react-dom 19.2.5→19.2.6, eslint 9→10, typescript 6.0.2→6.0.3, tailwindcss 4.2.2→4.2.4, @bufbuild/buf 1.67→1.69, @bufbuild/protobuf 2.11→2.12, autoprefixer 10.4→10.5, postcss 8.5.9→8.5.14.

## [0.41.1] - 2026-04-11

### Changed
- **Dependency updates across all three sub-projects**:
  - **Rust** (`Cargo.toml`): bumped pyo3 0.28.2→0.28.3, tokio 1.50→1.51, tokio-tungstenite 0.28→0.29, clap 4.5.60→4.6.0, uuid 1.22→1.23, image 0.25.9→0.25.10, swash 0.2.6→0.2.7, nix 0.31→0.31.2, proptest 1.10→1.11, tempfile 3.26→3.27, plus patch bumps (libc, subtle, zeroize, sysinfo, tar, tracing-subscriber, unicode-segmentation). No source changes required — all API-compatible.
  - **Python** (`pyproject.toml`): bumped pillow 12.1.1→12.2.0, maturin 1.12.6→1.13.1, pytest 9.0.2→9.0.3, rich 14.3.3→14.3.4, ruff 0.15.5→0.15.10.
  - **Frontend** (`web-terminal-frontend/package.json`): bumped next 16.1.6→16.2.3, react/react-dom 19.2.4→19.2.5, typescript 5.9.3→6.0.2 (major), @bufbuild/buf 1.66→1.67, @tailwindcss/postcss + tailwindcss 4.2.1→4.2.2, @types/node 25.3→25.6, postcss 8.5.8→8.5.9.

### Fixed
- **Flaky coprocess tests**: Replaced fixed `thread::sleep(200ms)` with a `poll_until(timeout_ms, fn)` helper (10ms poll interval, 2s deadline) throughout the coprocess test suite in both `src/coprocess.rs` and `tests/test_coprocess.rs`. Fixed-duration sleeps raced the OS scheduler under parallel CPU contention, causing intermittent `Some(true) vs Some(false)` status failures on loaded CI boxes. The full suite now passes 3/3 back-to-back runs. Affects: `test_coprocess_write_read`, `test_coprocess_feed_output`, `test_coprocess_dead_process`, `test_coprocess_stderr_capture`, `test_coprocess_auto_cleanup_never_policy`, `test_coprocess_restart_always_policy`, `test_coprocess_restart_on_failure_clean_exit`, `test_coprocess_restart_on_failure_nonzero_exit`, `test_coprocess_no_copy_output`.
- **Terminal.tsx reconnect forward-reference**: `scheduleRetry` called `connect()` before `connect` was declared (TDZ / `react-hooks/refs` violation). Introduced a `connectRef` forward-reference synced via `useEffect` so scheduled reconnects always invoke the current closure.
- **TerminalDebug.tsx ref-during-render**: The debug overlay read `debugLogs.current.length` during render, which React disallows. Replaced with a `logCount` state updated when logs are pushed.
- **page.tsx inline component definition**: `StatusIndicator` was defined inside the `Home` component body, so it was re-created every render and would reset its own state. Hoisted to module scope alongside a new `STATUS_CONFIG` table and now takes `status` as a prop.
- **Flaky `test_ioctl_returns_updated_size`**: Replaced fixed `time.sleep(0.3)` / `time.sleep(0.5)` windows with a `wait_for_either` poll loop (50ms interval, 5s deadline) that watches the subprocess log file for SIGWINCH / poll markers. Under CPU contention the fixed sleeps raced the scheduler and the spawned Python interpreter had not yet written its log line.
- **Slow `test_very_large_terminal`**: Added `@pytest.mark.timeout(30)` override — rendering a 200×100 (20k-cell) terminal to PNG legitimately takes 6-9s under CPU contention, exceeding the 5s global pytest timeout. The default stays tight for fast tests.

### Build / Tooling
- **Frontend lint migrated from `next lint` to standalone ESLint**: Next.js 16 removed the `next lint` subcommand. Added `eslint@^9.39.4` + `eslint-config-next@^16.2.3` as dev dependencies, replaced legacy `.eslintrc.json` with flat-config `eslint.config.mjs`, and updated the `lint` npm script to `eslint .`. Downgraded two compiler-focused `react-hooks` v7 rules (`set-state-in-effect`, `preserve-manual-memoization`) from `error` to `warn` — they are calibrated for React-Compiler codebases and fire false-positives on idiomatic Next.js SSR hydration and pre-Compiler useCallback patterns.

## [0.41.0] - 2026-03-11

### Added
- **`TriggerAction::SplitPane` and `ActionResult::SplitPane`**: New trigger action that instructs the frontend to open a split pane. Supports `direction` (`horizontal`/`vertical`), `focus_new_pane`, `target` (`active`/`source`), and optional `command` (either `SendText` or `InitialCommand`). Polled via `poll_action_results()` which returns dicts with `type="split_pane"`.
  - New supporting types: `TriggerSplitDirection`, `TriggerSplitTarget`, `TriggerSplitCommand` (all re-exported from `terminal` module)
  - Python `TriggerAction("split_pane", {...})` fully handled: `to_trigger_action()` parses all params; `poll_action_results()` serialises the result dict
  - Python binding `split_pane()` added to expose the action from Python

## [0.40.0] - 2026-03-08

### Added
- **VT100 ACS (Alternate Character Set) line-drawing support**: Applications like tmux that fall back from UTF-8 to ACS line-drawing now render correct box-drawing glyphs instead of raw ASCII letters.
  - `ESC ( 0` / `ESC ( B`: designate G0 charset as DEC Line Drawing / ASCII
  - `ESC ) 0` / `ESC ) B`: designate G1 charset as DEC Line Drawing / ASCII
  - `SO` (0x0E) / `SI` (0x0F): shift active charset to G1 / G0
  - Full 22-entry ACS→Unicode mapping applied in `write_char` when active charset is DecLineDrawing

### Fixed
- **Streaming `result_large_err` suppression**: Added self-documenting comments explaining why `#[allow(clippy::result_large_err)]` is used in WebSocket handshake callbacks — the `ErrorResponse` type is fixed by the tungstenite `Callback` trait and cannot be reduced or boxed.

## [0.39.8] - 2026-03-05

### Fixed
- **PTY env var stripping**: Fixed the re-apply loop to skip `DROP_VARS` so that stripped multiplexer env vars (TMUX, TMUX_PANE, STY, WINDOW, COLUMNS, LINES) are not re-added by the parent env re-application step.

## [0.39.7] - 2026-03-05

### Fixed
- **PTY env var stripping**: Fixed environment variable removal for multiplexer vars (TMUX, TMUX_PANE, STY, WINDOW, COLUMNS, LINES). The previous approach of skipping vars during iteration didn't work because `CommandBuilder::new()` pre-loads the full parent environment via `get_base_env()`. Now uses `env_remove()` to explicitly remove unwanted vars after base env is loaded.

## [0.39.6] - 2026-03-05

### Fixed
- **Streaming `screen_cleared` subscription**: Python clients could not subscribe to `"screen_cleared"` events via the streaming protocol because the string-to-EventType mapping was missing. The reverse conversion (EventType to string) worked, so events were emitted but unsubscribable.

### Docs
- Updated README, SECURITY.md, and CROSS_PLATFORM.md to document PTY multiplexer env var stripping (TMUX, TMUX_PANE, STY, WINDOW)

## [0.39.5] - 2026-03-04

### Added
- **`child_pid()` on `PtySession`**: New method that returns the PID of the spawned child process (shell or command) as `int | None`. Useful for process management (sending signals, monitoring, etc.).

### Fixed
- **PTY environment leakage**: Child PTY processes now strip tmux (`TMUX`, `TMUX_PANE`) and GNU Screen (`STY`, `WINDOW`) environment variables from the parent. Previously these leaked into spawned shells, causing tools like fzf to render in the parent multiplexer pane instead of the embedded PTY.

## [0.39.4] - 2026-03-04

### Added
- **ScreenCleared terminal event**: New event emitted when ED 2J (clear screen) or ED 3J (clear screen + scrollback) is received. Frontends can use this to invalidate scrollback zone/mark metadata so the scrollbar stays consistent with terminal state.
  - New `TerminalEvent::ScreenCleared { include_scrollback: bool }` variant
  - New `poll_screen_cleared_events()` method on Terminal to drain these events
  - Streaming protocol support with new `screen_cleared` message type and `ScreenCleared` event subscription
  - Python binding: `poll_screen_cleared_events()` returns `list[bool]`
- **OSC 133;C command extraction**: Shell integration now extracts the command text from `OSC 133;C;<command>` sequences sent by shell scripts before command execution markers.

### Changed
- **Login shell detection**: PTY session now detects `-l`/`--login` flags for shell spawning. (Note: portable-pty's CommandBuilder uses args[0] for both path resolution and arg0, so `$0` shows the shell path rather than `-bash`; the `-l` flag provides full login shell behavior regardless.)
- Added `nix` dependency (0.29) with process/term/signal features for Unix platforms

## [0.39.3] - 2026-02-25

### Fixed
- **Graphics scrollback clearing**: When clearing the terminal screen (ED 2) or screen including scrollback (ED 3), graphics stored in the scrollback buffer are now also properly cleared. Previously, only the active graphics were cleared, leaving orphaned graphics in scrollback.

## [0.39.2] - 2026-02-22

### Added
- **GitNexus MCP Integration**: Add code intelligence skills for exploring, debugging, impact analysis, and refactoring using the GitNexus knowledge graph
  - New skill files: `exploring/SKILL.md`, `debugging/SKILL.md`, `impact-analysis/SKILL.md`, `refactoring/SKILL.md`
  - Documentation added to `CLAUDE.md` and `AGENTS.md` with tool references and workflow guides
  - `.gitnexus` cache directory added to `.gitignore`

### Fixed
- **OSC52 empty clipboard writes**: Ignore empty clipboard write operations instead of processing them. Some applications send empty OSC52 sequences which should be no-ops.

## [0.39.1] - 2026-02-19

### Fixed
- **Increase OSC data size limit**: Raise `MAX_OSC_DATA_LENGTH` from 1 MB to 128 MB to support inline images via iTerm2/Kitty protocols. The previous limit silently dropped any image whose base64-encoded OSC sequence exceeded 1 MB (~750 KB raw).

## [0.39.0] - 2026-02-15

### Security
- **Constant-time auth comparisons (S-1, S-2)**: All API key and password comparisons now use `subtle::ct_eq()` to prevent timing attacks
- **API key query parameter disabled by default (S-3)**: Add `allow_api_key_in_query` config flag (default `false`) to `StreamingConfig` and CLI `--allow-api-key-in-query`. Query param API keys are logged by proxies and leaked via Referer headers; now opt-in only
- **Coprocess command injection prevention (S-4)**: Validate coprocess commands for shell metacharacters (`|;&$` etc.), path traversal (`..`), working directory traversal, and environment variable name format
- **Shell path validation (S-5)**: Validate `$SHELL` environment variable points to an existing file before use; fallback to `/bin/sh`
- **Image/graphics size limits (S-6 through S-9)**: Add `MAX_IMAGE_DIMENSION`, `MAX_IMAGE_DATA_SIZE`, `MAX_SIXEL_DIMENSION`, `MAX_SIXEL_COLORS`, and `GraphicsLimits` struct with configurable bounds for all graphics protocols
- **OSC string length limit (S-10)**: Add `MAX_OSC_DATA_LENGTH` (1MB) to reject oversized escape sequences
- **Clipboard size limit (S-11)**: Add `MAX_CLIPBOARD_CONTENT_SIZE` (10MB) to bound clipboard memory usage
- **TLS key permission validation (S-12)**: Warn on Unix when TLS private key file has group/world-readable permissions
- **Password file permission validation (S-13)**: Warn on Unix when htpasswd file has insecure permissions
- **Password memory zeroization (S-14)**: Add `zeroize` crate; `PasswordConfig` now zeroizes sensitive data on drop to prevent credential leakage in memory dumps
- **FFI safety documentation (S-15)**: Strengthen `SharedState::from_terminal()` safety contract documentation with pointer lifetime and exclusivity requirements
- **NaN handling in image size (Q-10)**: Image size `is_auto()` now handles NaN/Infinity correctly

### Fixed
- **Observer dispatch race (D-1)**: Fix duplicate event dispatch when `process()` is called multiple times before `poll_events()`. Events are now tracked with a dispatch index to prevent re-delivery to observers
- **Tab stop resize validation (Q-8)**: Guard tab stop array operations against zero-width terminal resize
- **Origin mode underflow (Q-9)**: Use `saturating_sub` in cursor positioning to prevent underflow when scroll region bounds are invalid
- **Scroll region return values (Q-7)**: `scroll_region_up()`/`scroll_region_down()` now return `bool` indicating success/failure
- **Clippy needless_return**: Fix clippy warning in `get_default_shell()`
- **Makefile: npm → bun**: Replace all `npm` commands with `bun` in web frontend targets (`web-install`, `web-dev`, `web-build`, `web-build-static`, `web-start`, `proto-typescript`) to match the project's actual package manager
- **Makefile: Add `test-rust-streaming` target**: Streaming Rust tests (69 additional tests) were never run; new target added to `test`, `checkall`, and help text
- **Missing `websockets` dev dependency**: Add `websockets` to `pyproject.toml` dev dependencies so streaming Python tests actually run instead of silently skipping
- **Streaming test skip guard**: Fix `test_streaming.py` skip logic to catch `RuntimeError`/`TypeError` (not just `ImportError`) when the streaming feature isn't compiled, so tests skip gracefully with `make dev`

### Added
- **Streaming server unit tests (T-1)**: 47 new tests for `validate_terminal_size()`, `HttpBasicAuthConfig::verify()`, `SessionRegistry` lifecycle, `StreamingConfig` defaults, and `ApiAuthConfig::is_configured()`
- **Recording system tests (T-5)**: 13 new tests for recording lifecycle, timestamp accuracy, event type recording, asciicast/JSON export format validation, and session ID uniqueness
- **Event system tests (T-4)**: 29 new tests for `TerminalEvent::kind()` exhaustive variant coverage, event queuing through `process()`/`poll_events()`, and event struct validation
- **HTML export tests (T-6)**: 20 new tests (up from 2) for text rendering, HTML escaping, text attributes (bold/italic/underline/strikethrough/dim/blink/hidden/reverse), color rendering, and wide characters
- **Search tests (T-8)**: 24 new tests (up from 2) for regex patterns, search options, match navigation, scrollback search, Unicode support, and API coverage
- **Python binding: `poll_upload_requests()`**: Add missing Python binding for `Terminal.poll_upload_requests()` to drain pending upload request events

### Changed
- **Dependencies**: Update all Rust, Python, and Node dependencies to latest versions. Notable: sysinfo 0.34→0.38, pyo3 0.28.1, ruff 0.15, maturin 1.12
- **MSRV**: Bump minimum supported Rust version from 1.75 to 1.88 (required by sysinfo 0.38)
- **Clippy auto-fix**: Replace `map_or(true, ...)` with `is_none_or(...)` in image deletion (Rust 1.82+)
- **CLAUDE.md**: Improve developer guidance with build commands, single-test examples, architecture overview, streaming protocol layers, and feature flags documentation

### Documentation
- **Instant Replay guide (DOC-1)**: New `docs/INSTANT_REPLAY.md` with SnapshotManager configuration, ReplaySession navigation, Python API reference, and memory tuning
- **C/C++ FFI guide (DOC-2)**: New `docs/FFI_GUIDE.md` with SharedCell/SharedState types, memory ownership rules, build instructions, and C code examples
- **Observer patterns guide (DOC-7)**: New `docs/OBSERVERS.md` with Rust/Python observer implementation, event categories, subscription filtering, and thread safety
- **Streaming Python examples (DOC-4)**: Add Python integration section to `docs/STREAMING.md` with StreamingConfig, TLS, and API key examples
- **README deduplication (DOC-5)**: Consolidate verbose "What's New" sections into brief summary pointing to CHANGELOG.md
- **BUILDING.md version fix (DOC-6)**: Remove stale version reference from title

## [0.38.0] - 2026-02-14

### Added
- **Instant Replay**: Add Instant Replay system with cell-level terminal snapshots, input-stream delta recording, and timeline navigation (Issue #47)
  - `TerminalSnapshot` and `GridSnapshot` structs capture complete terminal state (grids, cursors, colors, attributes, modes, scroll regions, tab stops) with memory size estimation
  - `Terminal::capture_snapshot()` and `Terminal::restore_from_snapshot()` for point-in-time state capture and restore
  - `SnapshotManager` manages a rolling buffer of snapshots with size-based eviction (default 4 MiB budget, 30-second interval), input-stream recording, `reconstruct_at()` for delta replay, and `find_entry_for_timestamp()` for timestamp-based lookup
  - `ReplaySession` provides timeline navigation with `seek_to()`, `step_forward()`, `step_backward()`, `seek_to_start()`, `seek_to_end()`, `seek_to_timestamp()`, `next_entry()`, and `previous_entry()`
  - Python binding: `capture_replay_snapshot()` returns dict with `timestamp`, `cols`, `rows`, `estimated_size_bytes`
- **General-Purpose File Transfer (OSC 1337 File= with inline=0)**: Full file download/upload support via the iTerm2 OSC 1337 `File=` protocol
  - New `FileTransfer` type and `FileTransferManager` with bounded ring buffer for completed transfers (default 32 entries, 50MB max size)
  - Downloads (`inline=0`): Base64 payload decoded, progress tracked, raw bytes stored for frontend retrieval via `take_completed_transfer()`
  - Multipart downloads (`MultipartFile`/`FilePart`): Chunked transfers routed through `FileTransferManager` with per-chunk progress events
  - Single-file downloads: Complete file received and stored in one step
  - Inline images (`inline=1`): Existing graphics path unchanged (no regression)
- **RequestUpload Protocol (OSC 1337 RequestUpload)**: Terminal-to-host file upload support
  - Host sends `RequestUpload=format=tgz`, terminal emits `upload_requested` event
  - Frontend responds via `send_upload_data(data)` (writes `ok\n` + base64 to PTY) or `cancel_upload()` (writes abort)
- **File Transfer Terminal Events**: Five new `TerminalEvent` variants: `FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`
  - All events routed to `on_screen_event()` in the observer system
  - New `EventKind` variants for subscription filtering: `FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`
- **Python Bindings: File Transfer API**: 9 new methods on both `Terminal` and `PtyTerminal`
  - Query: `get_active_transfers()`, `get_completed_transfers()`, `get_transfer(id)`
  - Retrieve: `take_completed_transfer(id)` (includes raw `data` bytes)
  - Control: `cancel_file_transfer(id)`, `send_upload_data(data)`, `cancel_upload()`
  - Config: `set_max_transfer_size(bytes)`, `get_max_transfer_size()`
- **Python Bindings: File Transfer Observer Events**: Observer callbacks receive `file_transfer_started`, `file_transfer_progress`, `file_transfer_completed`, `file_transfer_failed`, and `upload_requested` event dicts
- **Streaming Protocol: File Transfer Events**: Five new protobuf messages (`FileTransferStarted`, `FileTransferProgress`, `FileTransferCompleted`, `FileTransferFailed`, `UploadRequested`) and event types (`EVENT_TYPE_FILE_TRANSFER_STARTED=20` through `EVENT_TYPE_UPLOAD_REQUESTED=24`) for real-time WebSocket delivery
- **Python Bindings: Streaming File Transfer**: `encode_server_message()` and `decode_server_message()` support all 5 file transfer message types
- **Terminal Observer API**: New `TerminalObserver` trait enables push-based event delivery with deferred dispatch (events dispatched after `process()` returns). Supports category-specific callbacks (`on_zone_event`, `on_command_event`, `on_environment_event`, `on_screen_event`) plus catch-all `on_event`. Observer panic isolation via `catch_unwind` prevents one bad observer from crashing the terminal
- **Terminal Observer API: Subscription Filtering**: Observers can implement `subscriptions()` to receive only specific event kinds, avoiding unnecessary dispatch overhead
- **C-Compatible FFI**: New `SharedState` and `SharedCell` `#[repr(C)]` types provide a frozen snapshot of terminal state (dimensions, cursor, title, CWD, screen content with per-cell text/color/attributes) for C/C++ consumers. C API: `terminal_get_state()`, `terminal_free_state()`, `terminal_add_observer()`, `terminal_remove_observer()`
- **C FFI Observer Vtable**: `TerminalObserverVtable` struct with function pointers enables C consumers to register observers with `user_data` context pointer
- **Python Bindings: Sync Observer**: `Terminal.add_observer(callback, kinds=None)` registers a Python callable that receives event dicts after each `process()` call. Optional `kinds` parameter filters by event type
- **Python Bindings: Async Observer**: `Terminal.add_async_observer(kinds=None)` returns `(observer_id, asyncio.Queue)` tuple for async event consumption via `await queue.get()`
- **Python Bindings: Observer Management**: `Terminal.remove_observer(id)` and `Terminal.observer_count()` for observer lifecycle management
- **Python Convenience Wrappers**: `on_command_complete()`, `on_zone_change()`, `on_cwd_change()`, `on_title_change()`, `on_bell()` in `par_term_emu_core_rust.observers` module for common observer patterns

## [0.37.0] - 2026-02-13

### Added
- **Streaming Server: API Key Authentication**: New `api_key` field on `StreamingConfig` enables API key authentication for API routes (`/ws`, `/sessions`, `/stats`) while leaving static files (web frontend) unprotected. Accepted via `Authorization: Bearer <key>`, `X-API-Key: <key>` header, or `?api_key=<key>` query parameter. When both API key and HTTP Basic Auth are configured, either satisfies authentication. Wired from existing `--api-key` CLI flag (env: `PAR_TERM_API_KEY`). WebSocket-only server modes also validate auth during handshake
- **Streaming Server: Unified Auth Middleware**: New `ApiAuthConfig` struct and `api_auth_middleware` replace the old `basic_auth_middleware`, supporting both API key and HTTP Basic Auth in a single middleware. Auth is applied only to API routes via axum nested router pattern
- **Web Frontend: API Key Passthrough**: `getDefaultWsUrl()` now reads `api_key` from the page URL query params and appends it to the WebSocket URL, enabling `http://server:8099/?api_key=secret` to auto-authenticate the WS connection
- **Python Bindings: API Key Config**: `PyStreamingConfig` now exposes `api_key` as a constructor param (`api_key=None`) and getter/setter property. Masked as `api_key=***` in `__repr__`
- **Streaming Server: System Resource Statistics**: New optional system stats collection pushes CPU, memory, disk, network, and load average data to subscribed WebSocket clients. Enabled via `--enable-system-stats` CLI flag (env: `PAR_TERM_ENABLE_SYSTEM_STATS`) with configurable interval via `--system-stats-interval` (default 5s, env: `PAR_TERM_SYSTEM_STATS_INTERVAL`). Disabled by default
- **Streaming Server: Dedicated `/stats` Endpoint**: New WebSocket endpoint at `/stats` streams system stats as JSON to connected clients without requiring a terminal session. Requires `--enable-system-stats` flag. Provides CPU, memory, disk, network, load average, and host info at the configured interval
- **Streaming Protocol: `SystemStats` Message**: New `system_stats` server message type with nested `CpuStats`, `MemoryStats`, `DiskStats`, `NetworkInterfaceStats`, and `LoadAverage` structures. Includes static host info (hostname, OS name/version, kernel version) and dynamic metrics (CPU usage, memory, disk space, network I/O, load averages, uptime)
- **Streaming Protocol: `system_stats` Event Type**: New `EVENT_TYPE_SYSTEM_STATS = 18` for subscription filtering. Clients must subscribe to `system_stats` events to receive stats messages
- **Python Bindings: System Stats Config**: `PyStreamingConfig` now exposes `enable_system_stats` and `system_stats_interval_secs` as constructor params and getter/setter properties
- **Python Bindings: System Stats Decode**: `decode_server_message()` now returns full system stats data (cpu, memory, disks, networks, load_average, host info) as nested Python dicts/lists
- **Python Bindings: Missing Streaming Server Methods**: Added `send_cwd_changed()`, `send_trigger_matched()`, and `send_progress_bar_changed()` to `PyStreamingServer` for complete parity with Rust streaming server API
- **Kitty Graphics: Chunked Transmission**: Large images split across multiple DCS sequences are now properly accumulated and processed. Parser state persists on `Terminal` between chunks (`m=1` continues, `m=0` finalizes)
- **Kitty Graphics: Complete Delete Targets**: Implemented remaining `KittyDeleteTarget` variants — `AtCursor`, `InCell`, `OnScreen`, `ByColumn`, and `ByRow` now correctly remove graphics placements by position
- **Kitty Graphics: Placeholder Diacritics**: Unicode placeholder cells now include combining diacritics encoding row/column/MSB offsets in `Cell.combining`, enabling frontends to reconstruct full placeholder sequences
- **Screenshot: Synthetic Bold Rendering**: Bold text in screenshots is now visually emboldened using swash's `Render::embolden()` API (previously the `bold` parameter was accepted but ignored)
- **Screenshot: Font Load Failure Logging**: Emoji and CJK font load failures in the screenshot renderer now log error-level messages instead of being silently ignored

### Changed
- **Streaming Server: Auth middleware restructured** — `basic_auth_middleware` replaced by unified `api_auth_middleware` that handles both API key and HTTP Basic Auth. Auth is now applied only to API routes (`/ws`, `/sessions`, `/stats`) via nested router, leaving static file serving unprotected
- **Streaming: `PyPtyTerminal` methods gated with `#[cfg(feature = "streaming")]`** instead of `#[allow(dead_code)]` for clearer intent

### Removed
- Dead `DefaultSessionFactory` struct from streaming server (defined but never instantiated)
- Unused `advance_height` field from `GlyphMetrics` in screenshot font cache
- Unused `x_advance` and `y_advance` fields from `ShapedGlyph` in screenshot shaper

## [0.36.0] - 2026-02-11

### Added
- **Streaming Server: Per-Session Client Limits**: New `--max-clients-per-session` CLI flag and `PAR_TERM_MAX_CLIENTS_PER_SESSION` env var to cap concurrent clients per session (0 = unlimited). Enforced atomically via CAS loop in `try_add_client()`
- **Streaming Server: Input Rate Limiting**: New `--input-rate-limit` CLI flag and `PAR_TERM_INPUT_RATE_LIMIT` env var for per-client token bucket rate limiting (bytes/sec, 2x burst capacity). Applied to `Input` and `Paste` messages across all three WebSocket handlers (plain, TLS, Axum)
- **Streaming Server: Session Metrics**: New `SessionMetrics` struct tracks `messages_sent`, `bytes_sent`, `input_bytes`, `errors`, and `dropped_messages` per session with atomic counters. Metrics are included in `SessionInfo` for observability
- **Streaming Server: Terminal Size Validation**: `validate_terminal_size()` enforces bounds (2-1000 cols, 1-500 rows) on client resize requests and session creation. Invalid resize requests are logged and rejected
- **Streaming Server: Dead Session Reaping**: Session reaper now detects and cleans up sessions whose PTY process has exited and have no connected clients, via new `SessionFactory::is_session_alive()` trait method
- **Streaming Server: Broadcaster Health Check**: Reaper logs warnings when a session has active clients but no broadcast activity for 30+ seconds, aiding stalled broadcaster diagnosis
- **Streaming Server: `close_session()` Method**: New public method on `StreamingServer` handles session shutdown with delayed (500ms) factory teardown so clients receive the shutdown message
- **Streaming Server: WebSocket Query Parsing**: Plain and TLS listeners now use `accept_hdr_async` to capture URI query parameters during WebSocket handshake, enabling `?session=`, `?preset=`, and `?readonly` for non-Axum connections
- **Web Frontend: HyperlinkAdded Handler**: Terminal.tsx now handles `hyperlinkAdded` server messages, tracking hyperlinks by row and exposing an `onHyperlinkAdded` callback
- **Web Frontend: UserVarChanged Handler**: Terminal.tsx now handles `userVarChanged` server messages, maintaining a live Map of user variables and exposing an `onUserVarChanged` callback
- **Web Frontend: SelectionChanged Handler**: Terminal.tsx now handles `selectionChanged` server messages, syncing selection state to xterm.js (character and line modes) with automatic clipboard copy, and exposing an `onSelectionChanged` callback
- **Web Frontend: State Tracking**: page.tsx wires new callbacks to store hyperlinks (sliding window of 100) and user vars as React state for future UI consumption
- **Python Bindings: New Config Properties**: `PyStreamingConfig` now exposes `max_clients_per_session` and `input_rate_limit_bytes_per_sec` as constructor params and getter/setter properties
- **Shell Integration: `cursor_line` Field**: `TerminalEvent::ShellIntegrationEvent` now captures the absolute cursor line (`scrollback_len + cursor_row`) at the exact moment each OSC 133 marker is parsed. This enables correct per-marker positioning even when multiple markers arrive in a single frame
- **Shell Integration: `poll_shell_integration_events()`**: New convenience method on `Terminal` drains only shell integration events (keeping others queued), returning `ShellEvent` tuples with cursor position data
- **Shell Integration: `ShellEvent` Type Alias**: New `ShellEvent` type alias `(String, Option<String>, Option<i32>, Option<u64>, Option<usize>)` for typed shell event tuples
- **Streaming Protocol: `cursor_line` in `ShellIntegrationEvent`**: Protobuf and JSON protocol now include `cursor_line` field in shell integration events, propagated through all layers (proto, protocol, server, Python bindings)

### Fixed
- **Streaming Server: Shell Exit Deadlock**: Fixed potential deadlock when shell exits by dropping the PTY mutex guard before calling `close_session()`, and now properly notifies clients with a shutdown message
- **Streaming Server: PTY Write Error Handling**: All PTY write paths (input, mouse, focus, paste) now log errors and increment session error metrics instead of silently ignoring write failures

### Changed
- **BREAKING: `SessionState::try_add_client()`**: Now takes a `max_per_session: usize` parameter (0 = unlimited) instead of unconditionally accepting clients
- **BREAKING: `SessionInfo`**: Now includes five additional metrics fields (`messages_sent`, `bytes_sent`, `input_bytes`, `errors`, `dropped_messages`)
- **Streaming Server: Bounded Output Channel**: Output channel changed from `mpsc::unbounded_channel` to `mpsc::channel(1000)` for backpressure. All senders use `try_send()` instead of `send()`, dropping messages gracefully when the buffer is full
- **Streaming Server: Broadcast Metrics**: `SessionState::broadcast()` now tracks `messages_sent` and `dropped_messages` counters
- **Streaming Server: Idle Reaper Refactored**: Reaper now always runs (not gated by idle timeout config) to support dead session cleanup. Idle timeout reaping is conditional within the unified reaper loop

## [0.35.0] - 2026-02-10

### Fixed
- **Standalone Event Poller**: Fixed standalone mode's `poll_terminal_events()` silently dropping `ModeChanged`, `GraphicsAdded`, `HyperlinkAdded`, `UserVarChanged`, and `ProgressBarChanged` events via a `_ => {}` catch-all
- **HyperlinkAdded Event**: `TerminalEvent::HyperlinkAdded` now carries position data (`row`, `col`, `id`) and is actually emitted from the OSC 8 handler (was previously defined but never pushed to the event queue)
- **BREAKING: OSC 9;4 Progress Bar State Numbering**: Fixed `ProgressState` enum to match ConEmu/Windows Terminal spec - state 2 is now Error (was Indeterminate), state 3 is Indeterminate (was Warning), state 4 is Warning/Paused (was Error). Python `PyProgressState` discriminants updated to match
- **Python Streaming Bindings**: Added missing `encode_server_message` handlers for `cwd_changed`, `trigger_matched`, `user_var_changed`, and `progress_bar_changed` message types (decode already supported all variants)

### Added
- **XTVERSION Response**: Terminal now responds to `CSI > q` with `DCS > | par-term(version) ST`
- **DA1 OSC 52 Advertisement**: Primary Device Attributes response now includes parameter 52 to advertise OSC 52 clipboard support
- **Streaming Protocol: Mouse Input**: Clients can send mouse events (`MouseInput` message) with column, row, button, modifiers, and event type. Server translates to terminal escape sequences based on active mouse mode/encoding
- **Streaming Protocol: Focus Change**: Clients can send focus in/out events (`FocusChange` message). Server generates focus tracking escape sequences when focus tracking mode is active
- **Streaming Protocol: Paste Input**: Clients can send paste content (`PasteInput` message). Server wraps content in bracketed paste sequences when bracketed paste mode is active, or writes raw content otherwise
- **Streaming Protocol: Selection Sync**: Bidirectional selection synchronization via `SelectionChanged` (server→client) and `SelectionRequest` (client→server) messages supporting character, line, block, and word selection modes
- **Streaming Protocol: Clipboard Sharing**: Bidirectional clipboard access via `ClipboardSync` (server→client) and `ClipboardRequest` (client→server) messages for set/get operations with target support (clipboard, primary, select)
- **Streaming Protocol: Shell Integration Events**: `ShellIntegrationEvent` server message streams FinalTerm shell integration markers (`prompt_start`, `command_start`, `command_executed`, `command_finished`) with command text, exit codes, and timestamps
- **Streaming Protocol: Badge Changes**: `BadgeChanged` server message streams badge text updates from `OSC 1337 SetBadgeFormat` sequences
- **Streaming Protocol: Event Subscription**: `Subscribe` client message now fully implemented with per-client `HashSet<EventType>` filtering. Clients can subscribe to specific event types; unsubscribed events are filtered before broadcast. Applied in all 3 client loops (plain, TLS, Axum)
- **Streaming Server: New send_* Methods**: Added `send_mode_changed()`, `send_graphics_added()`, `send_hyperlink_added()`, `send_user_var_changed()`, `send_progress_bar_changed()`, `send_cursor_position()`, `send_badge_changed()`, `broadcast_to_session()` convenience methods to `StreamingServer`
- **Python Bindings: Streaming Server Methods**: All new `send_*` methods exposed on `PyStreamingServer`. New server/client message types supported in `encode`/`decode` functions
- **Web Frontend: Mouse Support**: Terminal.tsx now captures mouse events (click, release, move, scroll) and sends `MouseInput` messages when mouse tracking mode is active
- **Web Frontend: Focus Tracking**: Window focus/blur events sent as `FocusChange` messages when focus tracking mode is active
- **Web Frontend: Bracketed Paste**: Paste events intercepted and sent as `PasteInput` messages when bracketed paste mode is active
- **Web Frontend: Mode State Tracking**: `modeChanged` messages now update local state for `mouse_tracking`, `focus_tracking`, and `bracketed_paste` modes
- **New EventType Variants**: `Badge`, `Selection`, `Clipboard`, `Shell` added to subscription filtering system
- **New TerminalEvent Variants**: `BadgeChanged(Option<String>)`, `ShellIntegrationEvent { event_type, command, exit_code, timestamp }` added to core terminal event system

### Changed
- **BREAKING**: `TerminalEvent::HyperlinkAdded` changed from `HyperlinkAdded(String)` to struct variant `HyperlinkAdded { url: String, row: usize, col: usize, id: Option<u32> }`. All match sites must use struct destructuring
- **Protobuf Schema**: `proto/terminal.proto` expanded with 9 new message types and 4 new `EventType` enum values

## [0.34.0] - 2026-02-09

### Fixed
- **Terminal Mode Sync on Connect**: Clients connecting to existing streaming sessions now receive `ModeChanged` messages for all active non-default terminal modes (#31)
  - New `SessionState::build_mode_sync_messages()` sends mode state after `Connected` message in all WebSocket handlers (plain, TLS, Axum)
  - Synced modes: mouse tracking (x10/normal/button_event/any_event), mouse encoding (utf8/sgr/urxvt), bracketed paste, application cursor, focus tracking, cursor visibility, alternate screen, origin mode, insert mode, auto-wrap
  - Fixes mouse tracking and other modes not working when reconnecting to sessions where a TUI is already running
  - 16 new streaming integration tests, 13 new Rust unit tests

### Added
- **Terminal Mode Change Events**: DECSET/DECRST processing now emits `TerminalEvent::ModeChanged` events for real-time mode change broadcasting to connected clients
- **OSC 1337 RemoteHost**: Parse `RemoteHost=user@hostname` sequences for remote host integration (#29)
  - Supports `user@hostname` format (username is optional)
  - Updates `ShellIntegration` hostname and username fields
  - Treats `localhost`, `127.0.0.1`, and `::1` as local (clears hostname)
  - Emits `CwdChanged` event so frontends can react to remote host changes
  - Reuses existing streaming protocol `CwdChanged` message (no protocol changes needed)
  - `ShellIntegration` Python object now exposes `hostname` and `username` attributes
  - 14 Rust unit tests, 9 Python integration tests
- **OSC 934 Named Progress Bars**: Parse and manage multiple concurrent named progress bars (#22)
  - Protocol format: `OSC 934 ; action ; id [; key=value ...] ST` with `set`, `remove`, `remove_all` actions
  - Each bar has a unique ID, state (normal/indeterminate/warning/error), percentage (0-100), and optional label
  - New `named_progress_bars()`, `get_named_progress_bar(id)`, `set_named_progress_bar()`, `remove_named_progress_bar(id)`, `remove_all_named_progress_bars()` API (Rust and Python)
  - `ProgressBarChanged` terminal event emitted on create, update, and remove with action/id/state/percent/label
  - New `progress_bar_changed` streaming protocol message and `progress_bar` event type
  - Independent from existing OSC 9;4 single progress bar
  - 15 parser unit tests, 16 integration tests, 4 streaming tests, 17 Python integration tests
- **Unicode Normalization**: Configurable Unicode normalization (NFC/NFD/NFKC/NFKD) for text stored in terminal cells (#21)
  - New `NormalizationForm` enum with five forms: `None` (disabled), `NFC` (default), `NFD`, `NFKC`, `NFKD`
  - Terminal defaults to NFC (Canonical Composition) for consistent text storage
  - Normalization applied in VTE `print()` for decomposition and in `write_char()` for composition
  - New `normalization_form()` and `set_normalization_form(form)` Rust API
  - Python `NormalizationForm` enum (`Disabled`, `NFC`, `NFD`, `NFKC`, `NFKD`) with `Terminal.normalization_form()` and `Terminal.set_normalization_form()` methods
  - New `Cell::from_grapheme_normalized()` method for direct cell construction
  - 17 Rust unit tests, 13 Python integration tests
- **OSC 1337 SetUserVar**: Parse `SetUserVar=<name>=<base64_value>` sequences from shell integration scripts (#25)
  - Base64-decode values and store as user variables in terminal session state
  - New `get_user_var(name)` and `get_user_vars()` API (Rust and Python)
  - `UserVarChanged` terminal event emitted when a variable changes (includes old value)
  - User variables are accessible via badge session variables for format evaluation
  - New `user_var_changed` streaming protocol message and `user_var` event type
  - Python `poll_events()` / `poll_subscribed_events()` return `user_var_changed` event dicts
  - 9 Rust unit tests, 3 streaming protocol tests, 10 Python integration tests
- **Image Metadata Serialization**: Support for persisting and restoring graphics state with terminal sessions (#18)
  - New `serialization` module with `SerializableGraphic`, `GraphicsSnapshot`, and `ImageDataRef` types
  - `ImageDataRef` supports inline base64-encoded pixel data or external file path references for compact storage
  - `GraphicsStore.export_snapshot()` / `import_snapshot()` for full graphics state round-trip (placements, scrollback, animations)
  - `GraphicsStore.export_json()` / `import_json()` convenience methods for JSON serialization
  - Python `Terminal.export_graphics_json()` and `Terminal.import_graphics_json(json)` bindings
  - Added `Serialize`/`Deserialize` derives to `GraphicProtocol`, `ImageDisplayMode`, `ImageSizeUnit`, `ImageDimension`, `ImagePlacement`, `CompositionMode`, `AnimationState`, `AnimationControl`
  - Version-tagged snapshots (`GraphicsSnapshot.version`) for forward compatibility
- **Image Placement Metadata**: Parse and expose unified image placement modes from graphics protocols (#16)
  - New `ImagePlacement` struct with display mode, sizing, z-index, and sub-cell offset fields
  - New `ImageDimension` struct with unit support (auto, cells, pixels, percent)
  - **Kitty protocol**: Extracts columns/rows sizing (`c=`/`r=`), z-index for layering (`z=`), and sub-cell offsets (`x=`/`y=`)
  - **iTerm2 protocol**: Parses `width`/`height` with unit support (cells, `px`, `%`, auto), `preserveAspectRatio` flag, and `inline` flag
  - Exposed to Python via `Graphic.placement` property returning `ImagePlacement` object
  - New `ImagePlacement` and `ImageDimension` Python classes importable from the package
  - Enables frontends to implement inline/cover/contain rendering without protocol-specific logic
- **Original Image Dimensions**: All graphics protocols (Sixel, iTerm2, Kitty) now expose `original_width` and `original_height` on `TerminalGraphic` and Python `Graphic` objects
  - These preserve the original decoded pixel dimensions even when `width`/`height` change during animation
  - Enables frontends to calculate correct aspect ratios when scaling images to fit terminal cells
  - Python `Graphic.__repr__()` now includes `original_size=WxH`
- **Kitty Graphics Compression (o=z)**: Support for zlib-compressed image data in the Kitty graphics protocol
  - Parses the `o=z` transmission parameter to detect zlib-compressed payloads
  - Automatically decompresses data before pixel decoding (transparent to consumers)
  - Works with all transmission types: direct, file, temp file, and chunked transfers
  - New `was_compressed` metadata flag on `TerminalGraphic` for diagnostics/logging
  - Python `Graphic.was_compressed` property exposed for frontend diagnostics
  - 8 new Rust tests covering compression parsing, decompression, chunked transfers, and error handling
- **Python API**: `shell_integration_state()` accessor returns the live `ShellIntegration` state; remote-host changes surface to Python observers as `cwd_changed` events

### Changed
- **Dependencies**: Migrated to PyO3 0.28 from 0.23, updating all Python binding patterns to the latest API
- **Dependencies**: `flate2` is now a non-optional dependency (previously only available under `streaming` feature), required for Kitty `o=z` decompression
- **Dependencies**: Added `unicode-normalization` v0.1.25 for Unicode text normalization support
- **Dependencies**: Updated multiple dependency versions across the project

## [0.33.0] - 2026-02-06

### Added
- **Multi-Session Streaming Support**: The streaming server now supports multiple concurrent terminal sessions
  - New `SessionState` struct encapsulates per-session terminal, broadcast channels, PTY writer, and client tracking
  - New `SessionFactory` trait allows custom session creation (e.g., PTY-backed sessions in the binary server)
  - New `SessionRegistry` for managing active sessions with idle timeout reaping
  - New `ConnectionParams` struct for passing session/preset/client parameters during WebSocket upgrade
  - New `SessionInfo` struct exposes session metadata (id, client_count, created_at)
  - Clients connect to specific sessions via `?session=<id>` query parameter
  - New sessions are auto-created on first connection (or via preset with `?preset=<name>`)
  - Idle sessions (no connected clients) are automatically reaped after configurable timeout
- **Shell Presets**: Named shell presets allow clients to request specific shell environments
  - CLI: `--preset python=python3 --preset node=node`
  - Clients connect with `?preset=name` to spawn a session with that shell
- **Client Identity & Read-Only Mode**: Connected message now includes `client_id` and `readonly` fields
  - Each WebSocket client receives a unique identifier
  - Read-only status is communicated in the connection handshake
- **Streaming Config Extensions**: New configuration options for multi-session support
  - `max_sessions`: Maximum concurrent sessions (default: 10)
  - `session_idle_timeout`: Seconds before idle sessions are reaped (default: 900, 0 = never)
  - `presets`: HashMap of preset name → shell command
- **New Error Variants**: `MaxSessionsReached`, `SessionNotFound`, `InvalidPreset` in `StreamingError`
- **Python Bindings**: `StreamingConfig` gains `max_sessions` and `session_idle_timeout` getters/setters; `decode_server_message` includes `client_id` and `readonly` in Connected dict
- **New Public Exports**: `ConnectionParams`, `SessionFactory`, `SessionFactoryResult`, `SessionInfo`, `SessionRegistry`, `SessionState` from `streaming` module
- **Streaming Protocol: ModeChanged Events**: New `ModeChanged` message notifies clients when terminal modes change
  - Includes `mode` name (e.g., "cursor_visible", "mouse_tracking", "bracketed_paste") and `enabled` boolean
  - New `EVENT_TYPE_MODE` subscription type; Python subscription name: `"mode"`
- **Streaming Protocol: GraphicsAdded Events**: New `GraphicsAdded` message notifies clients when images are added to the terminal
  - Includes `row` position and optional `format` ("sixel", "iterm2", "kitty")
  - New `EVENT_TYPE_GRAPHICS` subscription type; Python subscription name: `"graphics"`
- **Streaming Protocol: HyperlinkAdded Events**: New `HyperlinkAdded` message notifies clients when OSC 8 hyperlinks are added
  - Includes `url`, `row`, `col`, and optional `id` from the OSC 8 protocol
  - New `EVENT_TYPE_HYPERLINK` subscription type; Python subscription name: `"hyperlink"`

### Changed
- **Breaking**: `StreamingConfig` has three new required fields: `max_sessions`, `session_idle_timeout`, `presets`
- **Breaking**: `ServerMessage::Connected` variant has two new fields: `client_id: Option<String>`, `readonly: Option<bool>`
- **Breaking**: `ServerMessage::connected_full()` constructor takes two additional parameters (`client_id`, `readonly`)
- **Breaking**: `StreamingServer` internals refactored from single-terminal to multi-session architecture
- Binary server (`par-term-streamer`) refactored to use `BinarySessionFactory` for per-session PTY management

## [0.32.0] - 2026-02-06

### Added
- **Coprocess Restart Policies**: Coprocesses can now automatically restart when they exit
  - New `RestartPolicy` enum: `Never` (default), `Always`, `OnFailure` (restart on non-zero exit)
  - Configurable restart delay via `restart_delay_ms` to prevent tight restart loops
  - Dead coprocesses with `Never` policy are automatically cleaned up from the manager
  - Restart logic runs during `feed_output()` polling cycle
- **Coprocess Stderr Capture**: Coprocess stderr is now captured in a separate buffer
  - New `read_coprocess_errors()` / `read_errors()` methods on `PtySession` and `CoprocessManager`
  - Stderr is read via a dedicated background thread (previously discarded)
- **Trigger Notify/MarkLine as Frontend Events**: `Notify` and `MarkLine` trigger actions now emit `ActionResult` events instead of directly calling internal notification/bookmark methods
  - Frontends receive `notify` and `mark_line` entries from `poll_action_results()` with trigger_id, allowing custom handling
  - `MarkLine` action now supports an optional `color` parameter as RGB tuple (e.g., `"color": "255,128,0"`)
- **Streaming Protocol: Action Result Events**: New `ActionNotify` and `ActionMarkLine` messages in the streaming protocol
  - Frontends subscribed to `action` events receive trigger-driven notifications and line marks
  - New protobuf messages: `ActionNotify`, `ActionMarkLine` with `Color` support
  - New `EVENT_TYPE_ACTION` subscription type
  - New server methods: `send_action_notify()`, `send_action_mark_line()`
- **Python Bindings**: Updated `CoprocessConfig` with `restart_policy` and `restart_delay_ms` parameters; added `read_coprocess_errors()` to `PtyTerminal`; added `send_action_notify()` and `send_action_mark_line()` to `StreamingServer`
- **Python API**: coprocess entry points `start_coprocess(config)` and `read_from_coprocess(cid)`; restart policies are set as strings `"never"` (default), `"always"`, `"on_failure"` (non-zero exit only)

### Changed
- **Breaking**: `CoprocessManager.feed_output()` now takes `&mut self` instead of `&self` (manages restart lifecycle)
- **Breaking**: `Notify` and `MarkLine` trigger actions no longer directly enqueue notifications or add bookmarks; they emit `ActionResult` events for frontend handling via `poll_action_results()`
- **Breaking**: `TriggerAction::MarkLine` now has an additional `color: Option<(u8, u8, u8)>` field

## [0.31.1] - 2026-02-05

### Fixed
- **Trigger Column Mapping**: `TriggerMatch.col` and `TriggerMatch.end_col` now correctly report grid column positions for text containing wide characters (CJK, emoji) and multi-byte UTF-8 characters
  - Previously, regex byte offsets were used directly, producing incorrect column values for non-ASCII text
  - New `byte_offsets_to_grid_cols()` converts regex byte offsets to proper grid column indices
  - New `build_char_to_grid_col_map()` builds character-to-grid-column mapping that accounts for wide character spacers and combining characters
  - `process_trigger_scans()` now passes the column mapping to `scan_line()` for accurate position reporting
  - Trigger highlights now correctly overlay the matched text even with wide/combining characters in the same row

## [0.31.0] - 2026-02-05

### Fixed
- **Streaming Server Event Dispatch**: Terminal events (bell, title change, CWD change, trigger matches) are now actually dispatched to streaming clients
  - Added `poll_terminal_events()` task to streaming server that polls terminal events at 20Hz
  - Bell events, title changes, resize events, CWD changes, and trigger matches are now broadcast to all connected WebSocket clients
  - Previously, broadcast helpers existed but were never called

### Added
- **Streaming Protocol: CWD Change Events (OSC 7)**: New `CwdChanged` message in the streaming protocol
  - Includes old_cwd, new_cwd, hostname, username, and timestamp fields
  - New `EVENT_TYPE_CWD` subscription type
- **Streaming Protocol: Trigger Match Events**: New `TriggerMatched` message in the streaming protocol
  - Includes trigger_id, row, col, end_col, text, captures, and timestamp fields
  - New `EVENT_TYPE_TRIGGER` subscription type
- **Streaming: Enhanced Connected Message**: Connection handshake now includes additional terminal state
  - `badge`: Current badge text (from OSC 1337 badge format)
  - `faint_text_alpha`: Dim text alpha for SGR 2 rendering (0.0-1.0)
  - `cwd`: Current working directory (from OSC 7)
  - `modify_other_keys`: Current modifyOtherKeys mode (0-2)
- **Streaming: New broadcast helpers**: `send_cwd_changed()` and `send_trigger_matched()` on `StreamingServer`
- **Triggers & Automation (Feature 18)**: Regex-based pattern matching on terminal output with automated actions
  - `TriggerRegistry` with `RegexSet` for efficient multi-pattern matching across terminal output
  - Trigger actions: Highlight (with optional expiry), Notify, MarkLine, SetVariable (core-handled); RunCommand, PlaySound, SendText (emitted as events for frontend)
  - Capture group substitution (`$1`, `$2`, etc.) in action parameters
  - Trigger highlight overlays with time-based expiry
  - `StopPropagation` action to short-circuit remaining actions
  - New methods: `add_trigger()`, `remove_trigger()`, `set_trigger_enabled()`, `list_triggers()`, `get_trigger()`, `poll_trigger_matches()`, `process_trigger_scans()`, `get_trigger_highlights()`, `clear_trigger_highlights()`, `clear_expired_highlights()`, `poll_action_results()`
  - New event: `TriggerMatched` in `poll_events()`
- **Coprocess Management**: Run external processes alongside terminal sessions
  - `CoprocessManager` for spawning, stopping, and communicating with coprocesses
  - Automatic terminal output piping to coprocess stdin (configurable per coprocess)
  - Line-buffered stdout reading via background reader threads
  - Integrated with PTY reader thread for automatic output feeding
  - New PTY methods: `start_coprocess()`, `stop_coprocess()`, `write_to_coprocess()`, `read_from_coprocess()`, `list_coprocesses()`, `coprocess_status()`
- **Python Bindings**: Full PyO3 bindings for triggers and coprocesses
  - New classes: `Trigger`, `TriggerAction`, `TriggerMatch`, `CoprocessConfig`
  - Trigger methods on `Terminal` class
  - Coprocess methods on `PtyTerminal` class
- **Python API**: trigger actions `run_command`, `send_text`, `play_sound`, `set_variable`, and `mark_line`; `CoprocessConfig` gains `copy_terminal_output`; `last_status` exposes the coprocess's last exit status

## [0.30.0] - 2026-02-04

### Added
- **modifyOtherKeys Protocol**: XTerm extension for enhanced keyboard input reporting
  - State tracking for modifyOtherKeys mode (0=disabled, 1=special keys, 2=all keys)
  - CSI sequence parsing: `CSI > 4 ; mode m` to set mode
  - Query support: `CSI ? 4 m` returns `CSI > 4 ; mode m` response
  - New methods: `modify_other_keys_mode()` getter, `set_modify_other_keys_mode()` setter
  - Mode resets on terminal reset and alternate screen exit
  - 9 new tests for modifyOtherKeys functionality
- **Faint Text Alpha**: Configurable alpha multiplier for SGR 2 (dim/faint) text
  - New `faint_text_alpha` field in Terminal (default: 0.5 for 50% dimming)
  - New methods: `faint_text_alpha()` getter, `set_faint_text_alpha(alpha)` setter
  - Values clamped to 0.0-1.0 range
  - Propagated to screenshot renderer for consistent rendering
  - Python bindings for both Terminal and PtyTerminal classes
- **Python API**: `drain_responses()` returns queued device-query replies (DA/DSR/DECRQM) as bytes

## [0.29.0] - 2026-02-04

### Added
- **OSC 7 Enhancements**: Percent-decoding, username/hostname parsing, port stripping, query/fragment removal, and path validation for `file://` URLs
- **Session Variable Sync**: OSC 7 now updates badge/session variables (`path`, `hostname`, `username`) so badge formats immediately reflect directory changes
- **CWD History Context**: CWD change log now records hostname and username; Python `CwdChange` exposes these fields
- **CWD Change Events**: New `TerminalEvent::CwdChanged` (and Python `cwd_changed` poll_events entry) fires on OSC 7 or manual `record_cwd_change`
- **Username Handling**: Shell integration stores optional username from `user@host` OSC 7 payloads

### Changed
- **API**: `record_cwd_change` now accepts optional `hostname` and `username` (defaults preserved); badge/session variables cleared when hostname/username unset
- **Dependencies**: Added `percent-encoding` and `url` crates for robust OSC 7 parsing

### Fixed
- **Badge Accuracy**: Badge variables `\(path)` and `\(hostname)` now stay in sync when updated via OSC 7
- **UTF-8 Paths**: Paths with spaces or Unicode characters from OSC 7 are correctly percent-decoded

## [0.28.0] - 2026-02-03

### Added
- **Badge Format Support (OSC 1337 SetBadgeFormat)**: iTerm2-style badge support for terminal overlays
  - New `badge` module with `SessionVariables` struct for session information
  - OSC 1337 SetBadgeFormat sequence parsing with base64-encoded format strings
  - Variable interpolation using `\(variable)` syntax (e.g., `\(username)@\(hostname)`)
  - Supports session prefix: `\(session.variable)` and direct: `\(variable)`
  - Built-in variables: `hostname`, `username`, `path`, `job`, `last_command`, `profile_name`, `tty`, `columns`, `rows`, `bell_count`, `selection`, `tmux_pane_title`, `session_name`, `title`
  - Custom variables via `set_custom(name, value)`
  - Security validation rejects shell injection patterns (`$()`, backticks, pipes, etc.)
  - Python bindings: `badge_format()`, `set_badge_format()`, `clear_badge_format()`, `evaluate_badge()`, `get_badge_session_variable()`, `set_badge_session_variable()`, `get_badge_session_variables()`
  - Session variables auto-sync with terminal state (title, dimensions, bell count)
  - Reference: [iTerm2 Badge Documentation](https://iterm2.com/documentation-badges.html)

### Fixed
- **Tmux Control Mode CRLF Handling**: Fixed parser to strip `\r` from `\r\n` line endings sent by tmux
- **Tmux Output Trailing Spaces**: Fixed `%output` notifications to preserve trailing spaces (regression from `.trim()` call)
- **OSC 133 Exit Code Parsing**: Fixed exit code extraction from `OSC 133 ; D ; <exit_code> ST` sequences

## [0.27.0] - 2026-02-01

### Added
- **Tmux Control Mode Auto-Detection**: Automatic detection and switching to tmux control mode
  - New `set_tmux_auto_detect(enabled)` method to enable/disable auto-detection
  - New `is_tmux_auto_detect()` method to check if auto-detection is enabled
  - Parser automatically switches to control mode when `%begin` notification is detected
  - Handles race conditions where tmux output arrives before `set_tmux_control_mode(True)` is called
  - When `set_tmux_control_mode(True)` is called, auto-detect is automatically enabled
  - Data before `%begin` is returned as `TerminalOutput` notification, allowing normal terminal display
  - Python bindings for `Terminal` class (PtyTerminal accesses via `terminal()` method)
  - Comprehensive Rust tests for auto-detection scenarios
- **Python API**: `is_tmux_control_mode()` reports whether tmux control mode is active

### Changed
- `set_tmux_control_mode(true)` now also enables auto-detection for better race condition handling

## [0.26.0] - 2026-02-01

### Added
- **Session Recording Python Exports**: `RecordingEvent` and `RecordingSession` classes now exported from Python module
  - Import directly: `from par_term_emu_core_rust import RecordingEvent, RecordingSession`
  - Previously these types were registered but not exported in `__init__.py`

- **RecordingSession Enhanced API**: New properties to access recorded events and environment
  - `session.events` - List of `RecordingEvent` objects for iterating over recorded events
  - `session.env` - Dict of environment variables captured at recording start (TERM, COLS, ROWS, etc.)
  - Helper methods: `get_size()` returns (cols, rows), `get_duration_seconds()` returns float

- **RecordingEvent Properties**: Full access to event data
  - `event.timestamp` - Milliseconds since recording start
  - `event.event_type` - "Input", "Output", "Resize", or "Marker"
  - `event.data` - Raw bytes of the event
  - `event.metadata` - Optional (cols, rows) for resize events
  - `event.get_data_str()` - Helper to decode data as UTF-8 string

- **PtyTerminal Recording Methods**: Added missing recording methods to match Terminal API
  - `record_output(data)` - Record output data bytes
  - `record_input(data)` - Record input data bytes
  - `record_resize(cols, rows)` - Record terminal resize event
  - `record_marker(label)` - Add marker/bookmark to recording
  - `get_recording_session()` - Get current active recording session
- **Python API**: `start_recording(title)` begins a session recording and `stop_recording()` returns the `RecordingSession`

### Changed
- **GitHub Workflows**: Added version consistency check that runs before all build jobs
  - Validates Cargo.toml, pyproject.toml, and __init__.py versions match
  - Fails fast before expensive builds if versions are out of sync
  - Added to both CI and deployment workflows

### Documentation
- Updated `docs/MACROS.md` with complete `RecordingSession` and `RecordingEvent` API documentation

## [0.25.0] - 2026-01-31

### Added
- **Configurable Unicode Width**: Full control over character width calculations for proper terminal alignment
  - New `UnicodeVersion` enum (Unicode9 through Unicode16, plus Auto) for version-specific width tables
  - New `AmbiguousWidth` enum (Narrow for Western, Wide for CJK) for East Asian Ambiguous characters
  - New `WidthConfig` class combining both settings with convenience constructors `WidthConfig.cjk()` and `WidthConfig.western()`
  - Terminal API: `width_config()`, `set_width_config()`, `set_ambiguous_width()`, `set_unicode_version()`, `char_width()`
  - Standalone functions: `char_width()`, `str_width()`, `char_width_cjk()`, `str_width_cjk()`, `is_east_asian_ambiguous()`
  - Python bindings for all new types and functions on both `Terminal` and `PtyTerminal`
  - Enables proper alignment for CJK text, Greek/Cyrillic letters, mathematical symbols, and box-drawing characters

## [0.24.0] - 2026-01-31

### Added
- **Configurable Unicode Width (Rust API)**: Add support for configuring the Unicode version used for character width calculations
  - New `UnicodeVersion` enum (Unicode9 through Unicode16, plus Auto) for version-specific width tables
  - New `AmbiguousWidth` enum (Narrow for Western, Wide for CJK) for East Asian Ambiguous characters
  - New `WidthConfig` struct combining both settings
  - Terminal API: `width_config()`, `set_width_config()`, `set_ambiguous_width()`, `set_unicode_version()`, `char_width()`
  - Standalone functions: `char_width()`, `str_width()`, `char_width_cjk()`, `str_width_cjk()`, `is_east_asian_ambiguous()`

## [0.23.0] - 2026-01-31

### Added
- **Configurable ENQ Answerback**: Terminal can now return a custom answerback string in response to ENQ (0x05)
  - New Rust APIs: `Terminal::answerback_string()` and `Terminal::set_answerback_string()`
  - Python bindings expose `answerback_string()` and `set_answerback_string()` on both `Terminal` and `PtyTerminal`
  - Disabled by default for security; answerback payload is delivered via the existing response buffer (`drain_responses()`)

### Fixed
- **Python Version Sync**: Bumped Python package version to match crate release and expose new answerback feature

## [0.22.1] - 2026-01-30

### Fixed
- **Search Unicode Bug**: Fixed `search()` and `search_scrollback()` returning byte offsets instead of character offsets for multi-byte Unicode text
  - `SearchMatch.col` now correctly returns the character (grapheme) column position, not the byte offset
  - `SearchMatch.length` now correctly returns the character count, not the byte length
  - `SearchMatch.text` now correctly extracts the matched text using character iteration
  - Affects text containing multi-byte characters (CJK, emoji, etc.)
  - Example: Searching for "World" in "こんにちは World" now returns `col=6` (correct) instead of `col=16` (byte offset)
  - Added comprehensive tests for Unicode search scenarios

## [0.22.0] - 2026-01-27

### Added
- **Regional Indicator Flag Emoji Support**: Proper grapheme cluster handling for flag emoji
  - Flag emoji like 🇺🇸, 🇬🇧, 🇯🇵 are now correctly combined into single cells
  - Two regional indicator codepoints are combined into one wide (2-cell) grapheme
  - Flags are stored with the first indicator as the base character and the second in the combining vector
  - Cursor correctly advances by 2 cells after writing a flag
  - Added `unicode-segmentation` crate dependency for grapheme cluster support
  - Comprehensive test suite for flag emoji in `tests/test_flag_emoji.rs`

### Fixed
- **Clippy Warning**: Fixed unnecessary unwrap warning in screenshot font_cache.rs

## [0.21.0] - 2026-01-20

### Changed
- **Migrated to `parking_lot::Mutex`**: Replaced all `std::sync::Mutex` usage with `parking_lot::Mutex` for improved performance and reliability
  - Eliminated Mutex poisoning risk across the entire library, including Python bindings and streaming server
  - Simplified lock acquisition by removing `.unwrap()` calls on lock results
  - Smaller mutex memory footprint (1 byte vs system-dependent size)
  - Faster lock/unlock operations under contention

## [0.20.1] - 2026-01-20

### Added
- **Safe Environment Variable API for Spawn Methods** (Issue #13): New methods to pass environment variables directly to spawned processes without modifying the parent process environment
  - `spawn_shell_with_env(env, cwd)` - Rust API to spawn shell with env vars and working directory
  - `spawn_with_env(command, args, env, cwd)` - Rust API to spawn command with env vars and working directory
  - Python `spawn_shell(env=None, cwd=None)` - Updated signature to accept optional env dict and cwd string
  - Safe for multi-threaded applications (Tokio) - no `unsafe { std::env::set_var() }` required
  - Backward compatible - existing code calling `spawn_shell()` without args still works
  - Env vars from method parameters override those from `set_env()` (applied last)

### Documentation
- Updated README.md with examples for the new env/cwd parameters

## [0.20.0] - 2025-12-23

### Added
- **External UI Theme File**: Web frontend UI chrome theme can now be customized after static build
  - New `theme.css` file in `web_term/` directory contains CSS custom properties
  - Edit colors without rebuilding: `--terminal-bg`, `--terminal-surface`, `--terminal-border`, `--terminal-accent`, `--terminal-text`
  - Changes take effect on page refresh - no rebuild required
  - Terminal emulator colors (ANSI palette) still controlled by server `--theme` option

### Fixed
- **Web Terminal On-Screen Keyboard Mobile Fix**: Fixed native device keyboard appearing when tapping on-screen keyboard buttons on mobile
  - Removed `focusTerminal()` call after on-screen keyboard input to prevent xterm's internal textarea from triggering native keyboard
  - Added active element blur on touch to ensure no input retains focus
  - Only focus terminal when hiding on-screen keyboard, not when showing or using it

### Changed
- **Theme Architecture**: Separated UI chrome theme from terminal emulator theme
  - UI chrome (status bar, buttons, containers) now uses external `theme.css`
  - Terminal emulator colors continue to be sent from server via protobuf

### Documentation
- Updated `docs/STREAMING.md` with new "UI Chrome Theme" section
- Updated `web-terminal-frontend/README.md` with theme customization guide
- Added theme customization to main README features list

## [0.19.5] - 2025-12-17

### Fixed
- **Streaming Server Shell Restart Input**: Fixed WebSocket client connections not receiving input after shell restart
  - PTY writer was captured once at connection time, becoming stale after shell restart
  - Now fetches the latest PTY writer each time input needs to be written
  - Ensures client keyboard input reaches the shell after any restart

## [0.19.4] - 2025-12-17

### Added
- **Python SDK Sync with Rust SDK**: Aligned Python streaming bindings with all Rust streaming features
  - `StreamingConfig.enable_http` - Enable/disable HTTP static file serving (getter/setter)
  - `StreamingConfig.web_root` - Web root directory for static files (getter/setter)
  - `StreamingServer.max_clients()` - Get maximum number of allowed clients
  - `StreamingServer.create_theme_info()` - Static method to create theme dictionaries for protocol functions
  - `encode_server_message("pong")` - Added missing pong message type support
  - `encode_server_message("connected", theme=...)` - Added theme support with name, background, foreground, normal (8 colors), bright (8 colors)
- Streaming sessions are identified by `session_id` in the Python API

### Changed
- `StreamingConfig` constructor now accepts `enable_http` and `web_root` parameters (with backwards-compatible defaults)
- `StreamingConfig.__repr__()` now includes `enable_http` and `web_root` in output
- Updated deprecated `Python::with_gil` to `Python::attach` for PyO3 0.27 compatibility

## [0.19.3] - 2025-12-17

### Fixed
- **Shell Restart Hang**: Fixed streaming server hanging when attempting to restart the shell after exit
  - Added `cleanup_previous_session()` method to properly clean up old PTY resources before spawning new shell
  - Old writer is dropped first to unblock any blocked reads in the old reader thread
  - Old PTY pair is closed before creating new one
  - Old reader thread is waited on (with 2-second timeout) to ensure it finishes
  - Old child process is properly reaped to prevent zombie processes
  - Added detailed logging to shell restart process for easier debugging

### Security
- **Removed username from startup logs**: Streaming server no longer logs the HTTP Basic Auth username
  - Addresses CodeQL alert for cleartext logging of sensitive information (CWE-312, CWE-359, CWE-532)
  - Auth status still displayed as "ENABLED" or "DISABLED" without credential details

## [0.19.2] - 2025-12-17

### Fixed
- **Streaming Server Hang on Shell Exit**: Fixed server hanging indefinitely when the shell exits
  - Added shutdown signal mechanism using `tokio::sync::Notify` to gracefully terminate the broadcaster loop
  - The `output_broadcaster_loop` now listens for shutdown signals in its `select!` block
  - The existing `shutdown()` method now also signals the broadcaster to exit
  - Prevents the server from blocking indefinitely on `rx.recv()` when `output_tx` sender is never dropped

## [0.19.1] - 2025-12-16

### Fixed
- **Streaming Server Ping/Pong**: Fixed application-level ping/pong handling in the streaming server
  - Server was incorrectly sending WebSocket-level pong frames instead of protobuf `Pong` messages
  - Added `Pong` variant to `ServerMessage` protocol enum
  - Frontend heartbeat mechanism now properly receives pong responses
  - Fixes stale connection detection that was always failing due to missing pong responses

## [0.19.0] - 2025-12-16

### Added
- **Automatic Shell Restart**: Streaming server now automatically restarts the shell when it exits
  - Default behavior: shell is restarted automatically when it exits
  - New `--no-restart-shell` CLI option to disable automatic restart
  - New `PAR_TERM_NO_RESTART_SHELL` environment variable support
  - When restart is disabled, server exits when the shell exits
  - Shell restart preserves the PTY writer connection to streaming clients

- **Header/Footer Toggle in On-Screen Keyboard**: New layout toggle button in the keyboard header
  - Allows users to show/hide the header and footer directly from the on-screen keyboard
  - Visual indicator shows current state (blue when header/footer is visible)
  - Convenient for mobile users who want to maximize terminal space without closing the keyboard

- **Font Size Controls in On-Screen Keyboard**: Plus/minus buttons in keyboard header
  - Adjust terminal font size (8px to 32px) directly from the on-screen keyboard
  - Shows current font size between buttons
  - Buttons disabled at min/max limits

### Changed
- **StreamingServer Interior Mutability**: `set_pty_writer` now uses `&self` instead of `&mut self`
  - Enables updating PTY writer after shell restart without requiring mutable reference
  - Uses `RwLock` for thread-safe interior mutability

- **Web Frontend UI Improvements**:
  - Moved font size controls from main header to on-screen keyboard header
  - Repositioned floating toggle buttons side by side in bottom-right corner
  - Keyboard and header/footer toggle buttons now have consistent sizing

## [0.18.2] - 2025-12-15

### Added
- **Font Size Control**: User-adjustable terminal font size in web frontend
  - Plus/minus buttons in header to adjust font size (8px to 32px range)
  - Current font size displayed between buttons
  - Setting persisted to localStorage across sessions
  - Overrides automatic responsive sizing when set

- **Heartbeat/Ping Mechanism**: Stale WebSocket connection detection with automatic reconnection
  - Sends ping every 25 seconds, expects pong within 10 seconds
  - Closes and triggers reconnect on stale connections
  - Prevents "Connected" status showing for half-open sockets

### Security
- **Web Terminal Security Hardening**: Comprehensive security audit fixes for the web frontend
  - **Reverse-tabnabbing prevention**: Terminal links now open with `noopener,noreferrer` to prevent malicious links from hijacking the parent tab
  - **Zip bomb protection**: Added decompression size limits (256KB compressed, 2MB decompressed) to prevent memory exhaustion attacks
  - **Localhost probe fix**: WebSocket preconnect hints now gated to development mode only, preventing production sites from scanning localhost ports
  - **Snapshot size guard**: Added 1MB limit on screen snapshots to prevent UI freezes from oversized payloads

### Fixed
- **WebSocket URL Changes**: Changing the WebSocket URL while connected now properly disconnects and reconnects to the new server
- **Invalid URL Handling**: Invalid WebSocket URLs no longer crash the UI; displays friendly error message instead
- **Next.js Config Conflict**: Merged duplicate config files (`next.config.js` and `next.config.mjs`) into single file with `reactStrictMode` enabled
- **Toggle Button Overlap**: Moved header/footer toggle button left to avoid overlapping with scrollbar

## [0.18.1] - 2025-12-15

### Fixed
- **Web Terminal On-Screen Keyboard**: Fixed device virtual keyboard appearing when tapping on-screen keyboard buttons on mobile devices
  - Added `tabIndex={-1}` to all buttons in the on-screen keyboard component to prevent focus acquisition
  - Affects all keyboard sections: main keys, arrow keys, Ctrl shortcuts, symbol grid, macro buttons, and all UI controls

## [0.18.0] - 2025-12-14

### Added
- **Environment Variable Support**: All CLI options now support environment variables with `PAR_TERM_` prefix
  - Examples: `PAR_TERM_HOST`, `PAR_TERM_PORT`, `PAR_TERM_THEME`, `PAR_TERM_HTTP_USER`
  - Enabled via clap's `env` feature

- **HTTP Basic Authentication**: New password protection for the web frontend
  - `--http-user` - Username for HTTP Basic Auth
  - `--http-password` - Clear text password (env: `PAR_TERM_HTTP_PASSWORD`)
  - `--http-password-hash` - htpasswd format hash supporting bcrypt ($2y$), apr1 ($apr1$), SHA1 ({SHA}), MD5 crypt ($1$)
  - `--http-password-file` - Read password from file (auto-detects hash vs clear text)
  - Uses `htpasswd-verify` crate for hash verification

- **Comprehensive Streaming Test Suite**: 94 new tests for streaming functionality
  - Integration tests (`tests/test_streaming.rs`): Protocol message constructors, theme info, HTTP Basic Auth, StreamingConfig, binary protocol encoding/decoding, event types, streaming errors, JSON serialization
  - Unit tests in `broadcaster.rs`: Default implementation, client management, empty broadcaster operations
  - Unit tests in `proto.rs`: All message type encoding/decoding, Unicode content, ANSI escape sequences, event type conversions

### Changed
- **Dependencies**: Added `htpasswd-verify` and `headers` crates for HTTP Basic Auth support
- **Streaming Server**: Added `HttpBasicAuthConfig` and `PasswordConfig` types to `StreamingConfig`
- **Python Bindings**: Added exports for binary protocol functions (`encode_server_message`, `decode_server_message`, `encode_client_message`, `decode_client_message`) to `__init__.py`
- **Python Package Version**: Updated to 0.18.0 to match Cargo.toml

## [0.17.0] - 2025-12-13

### Added
- **Web Terminal Macro System**: New macro tab in the on-screen keyboard for creating and playing terminal command macros
  - Create named macros with multi-line scripts (one command per line)
  - Quick select buttons to run macros with a single tap
  - Playback with 200ms delay before each Enter key for reliable command execution
  - Edit and delete existing macros via hover menu
  - Stop button to abort macro playback mid-execution
  - Macros persist to localStorage across sessions
  - Visual feedback during playback (pulsing animation, stop button)
  - Option to disable sending Enter after each line (for text insertion macros)
  - Template commands for advanced macro scripting:
    - `[[delay:N]]` - Wait N seconds
    - `[[enter]]` - Send Enter key
    - `[[tab]]` - Send Tab key
    - `[[esc]]` - Send Escape key
    - `[[space]]` - Send Space
    - `[[ctrl+X]]` - Send Ctrl+X
    - `[[shift+X]]` - Send Shift+X (uppercase)
    - `[[ctrl+shift+X]]` - Send Ctrl+Shift+X
    - `[[shift+tab]]` - Reverse Tab
    - `[[shift+enter]]` - Shift+Enter

- **On-Screen Keyboard Enhancements**:
  - Permanent symbols grid on the right side with all keyboard symbols (32 keys)
  - Added Space and Enter buttons to modifier row
  - Added http:// and https:// quick insert buttons to modifier row
  - Added tooltips to Ctrl shortcut buttons explaining each shortcut
  - Expanded symbol keys: added `! @ # $ % ^ & * - _ = + : ; ' " , . ?`

### Changed
- **Web Frontend Dependencies**: Updated @types/node (25.0.1 → 25.0.2)
- **On-Screen Keyboard Layout**: Reorganized for better usability
  - Symbols now displayed as persistent grid instead of toggle row
  - Removed redundant Escape key from function key row
  - More compact vertical layout with reduced gaps

## [0.16.3] - 2025-12-08

### Fixed
- **Web Terminal: tmux/TUI DA Response Echo**: Fixed control characters (`^[[?1;2c^[[>0;276;0c`) appearing when running tmux or other TUI applications in the web terminal
  - Root cause: xterm.js frontend was generating Device Attributes (DA) responses when it received DA queries forwarded from the backend terminal
  - Solution: Registered xterm.js parser handlers to suppress DA1, DA2, DA3, and DSR responses (backend terminal emulator handles these)
  - Affected sequences: `CSI c` (DA1), `CSI > c` (DA2), `CSI = c` (DA3), `CSI n` (DSR), `CSI ? Ps $ p` (DECRQM)

### Added
- **jemalloc Allocator Support**: Optional `jemalloc` feature for 5-15% server throughput improvement
  - New Cargo feature: `jemalloc` (enabled separately from `streaming`)
  - Only available on non-Windows platforms (Unix/Linux/macOS)
  - Uses `tikv-jemallocator` v0.6

### Changed
- **Streaming Server Performance Optimizations**:
  - **TCP_NODELAY**: Disabled Nagle's algorithm on WebSocket connections for lower keystroke latency (up to 40ms improvement)
  - **Output Batching**: Time-based batching with 16ms window (60fps) reduces WebSocket message overhead by 50-80% during burst output
  - **Compression Threshold**: Lowered from 1KB to 256 bytes to compress more typical terminal output (prompts, short commands are 200-800 bytes)

- **Web Frontend Performance Optimizations**:
  - **WebSocket Preconnect**: Added preconnect hints for ws:// and wss:// to reduce initial connection latency by 100-200ms
  - **Font Preloading**: Preload JetBrains Mono to avoid layout shift and font flash

- **Web Frontend Dependencies**: Updated Next.js (16.0.7 → 16.0.8), @types/node (24.10.1 → 24.10.2)
- **Pre-commit Hooks**: Updated ruff (0.14.4 → 0.14.8)

## [0.16.2] - 2025-12-05

### Fixed
- **TERM Environment Variable**: Changed default `TERM` from `xterm-kitty` to `xterm-256color` for better compatibility with systems lacking kitty terminfo

## [0.16.1] - 2025-12-03

### Fixed
- **`cargo install` No Longer Requires `protoc`**: Pre-generated Protocol Buffer code is now included in the crate, eliminating the need to install the `protoc` compiler when building with the `streaming` feature
- Removed `prost-build` from default build dependencies (moved to optional `regenerate-proto` feature)
- CI workflow updated to remove unnecessary `protoc` installation steps

### Changed
- Protocol Buffer Rust code is now pre-generated in `src/streaming/terminal.pb.rs`
- Added new `regenerate-proto` feature for regenerating protobuf code from `proto/terminal.proto`

## [0.16.0] - 2025-12-03

### Changed
- **BREAKING: Binary Protocol for WebSocket Streaming**:
  - Replaced JSON-based WebSocket protocol with Protocol Buffers binary encoding
  - ~80% reduction in message sizes for typical terminal output
  - Optional zlib compression for payloads over 1KB (screen snapshots)
  - Wire format: 1-byte header (0x00=uncompressed, 0x01=compressed) + protobuf payload
  - Text WebSocket messages are no longer supported (binary only)

### Added
- **TLS/SSL Support for Streaming Server**:
  - New CLI options: `--tls-cert`, `--tls-key`, `--tls-pem` for enabling HTTPS/WSS
  - Supports separate certificate and key files or combined PEM file
  - Enables secure connections for production deployments
  - New `TlsConfig` struct in Rust API for programmatic TLS configuration

- **Protocol Buffers Infrastructure**:
  - New `proto/terminal.proto` schema file (single source of truth)
  - Rust code generation via `prost` + `prost-build` in `build.rs`
  - TypeScript code generation via `@bufbuild/protobuf` + `buf`
  - New `src/streaming/proto.rs` module for encode/decode with compression
  - New `lib/protocol.ts` helper module for frontend

- **Python Bindings for TLS and Binary Protocol**:
  - `StreamingConfig.set_tls_from_files(cert_path, key_path)` - Configure TLS from separate files
  - `StreamingConfig.set_tls_from_pem(pem_path)` - Configure TLS from combined PEM file
  - `StreamingConfig.tls_enabled` property - Check if TLS is configured
  - `StreamingConfig.disable_tls()` - Clear TLS configuration
  - `encode_server_message(type, **kwargs)` - Encode server messages to protobuf
  - `decode_server_message(data)` - Decode server messages from protobuf
  - `encode_client_message(type, **kwargs)` - Encode client messages to protobuf
  - `decode_client_message(data)` - Decode client messages from protobuf

- **Makefile Targets**:
  - `make proto-generate` - Generate protobuf code for Rust and TypeScript
  - `make proto-rust` - Generate Rust protobuf code only
  - `make proto-typescript` - Generate TypeScript protobuf code only
  - `make proto-clean` - Clean generated protobuf files

### Dependencies
- Added `prost` v0.14.1 (Rust protobuf runtime)
- Added `prost-build` v0.14.1 (Rust protobuf codegen, build dependency)
- Added `@bufbuild/protobuf` v2.10.1 (TypeScript protobuf runtime)
- Added `@bufbuild/protoc-gen-es` v2.10.1 (TypeScript protobuf codegen)
- Added `@bufbuild/buf` v1.61.0 (Protocol Buffers toolchain)
- Added `pako` v2.1.0 (TypeScript zlib compression)
- Added `rustls` v0.23.35 (TLS implementation)
- Added `tokio-rustls` v0.26.4 (Async TLS for Tokio)
- Added `rustls-pemfile` v2.2.0 (PEM file parsing)
- Added `axum-server` v0.7.3 (HTTPS server support)

## [0.15.0] - 2025-12-02

### Added
- **Streaming Server CLI Enhancements**:
  - `--download-frontend` option to download prebuilt web frontend from GitHub releases
  - `--frontend-version` option to specify version to download (default: "latest")
  - `--use-tty-size` option to use current terminal size from TTY for the streamed session
  - No longer requires Node.js/npm to use web frontend - can download prebuilt version

- **Web Terminal Onscreen Keyboard Improvements**:
  - Added Ctrl+Space shortcut (NUL character) for set-mark/autocomplete functionality

### Changed
- Documentation updated with new quick start using downloaded frontend
- Build instructions updated with `--no-default-features` flag

## [0.14.0] - 2025-12-01

### Added
- **Web Terminal Onscreen Keyboard**: Mobile-friendly virtual keyboard for touch devices
  - Special keys missing from iOS/Android keyboards: Esc, Tab, arrow keys, Page Up/Down, Home, End, Insert, Delete
  - Function keys F1-F12 (toggleable panel)
  - Symbol keys often hard to type on mobile: |, \, `, ~, {, }, [, ], <, >
  - Modifier keys: Ctrl, Alt, Shift (toggle to combine with other keys)
  - Quick Ctrl shortcuts: ^C, ^D, ^Z, ^L, ^A, ^E, ^K, ^U, ^W, ^R
  - Glass morphism design matching terminal aesthetic
  - Haptic feedback on supported devices
  - Auto-shows on mobile devices, toggleable on desktop
  - Proper ANSI escape sequence generation for all keys

- **OSC 9;4 Progress Bar Support** (ConEmu/Windows Terminal style):
  - New `ProgressState` enum with states: `Hidden`, `Normal`, `Indeterminate`, `Warning`, `Error`
  - New `ProgressBar` struct with `state` and `progress` (0-100) fields
  - Terminal methods: `progress_bar()`, `has_progress()`, `progress_value()`, `progress_state()`, `set_progress()`, `clear_progress()`
  - Full Python bindings for `ProgressState` enum and `ProgressBar` class
  - OSC 9;4 sequence parsing: `ESC ] 9 ; 4 ; state [; progress] ST`
  - Progress values are automatically clamped to 0-100

### Protocol Support
- **OSC 9;4 Format**:
  - `ESC ] 9 ; 4 ; 0 ST` - Hide progress bar
  - `ESC ] 9 ; 4 ; 1 ; N ST` - Normal progress at N%
  - `ESC ] 9 ; 4 ; 2 ST` - Indeterminate/busy indicator
  - `ESC ] 9 ; 4 ; 3 ; N ST` - Warning progress at N%
  - `ESC ] 9 ; 4 ; 4 ; N ST` - Error progress at N%

## [0.13.0] - 2025-11-27

### Added
- **Streaming Server Enhancements**:
  - `--size` CLI option for specifying terminal size in `COLSxROWS` format (e.g., `--size 120x40` or `-s 120x40`)
  - `--command` / `-c` CLI option to execute a command after shell startup (with 1 second delay for prompt settling)
  - `initial_cols` and `initial_rows` configuration options in `StreamingConfig` for both Rust and Python APIs

- **Python Bindings Enhancements**:
  - New `MouseEncoding` enum (`Default`, `Utf8`, `Sgr`, `Urxvt`) for mouse event encoding control
  - Screen buffer control: `use_alt_screen()`, `use_primary_screen()` for direct screen switching
  - Mouse encoding: `mouse_encoding()`, `set_mouse_encoding()` for controlling mouse event format
  - Mode setters: `set_focus_tracking()`, `set_bracketed_paste()` for direct mode control
  - Title control: `set_title()` for programmatic title changes
  - Bold brightening: `bold_brightening()`, `set_bold_brightening()` for legacy terminal behavior
  - Color getters: `link_color()`, `bold_color()`, `cursor_guide_color()`, `badge_color()`, `match_color()`, `selection_bg_color()`, `selection_fg_color()`
  - Color flag getters: `use_bold_color()`, `use_underline_color()`
- **Python API**: `set_faint_text_alpha(alpha)` / `faint_text_alpha` control dim-text opacity (0.0-1.0)

### Changed
- `StreamingConfig` now includes `initial_cols` and `initial_rows` fields (default: 0, meaning use terminal's current size)

## [0.12.0] - 2025-11-27

### Fixed
- **Terminal Reflow Improvements**: Multiple fixes to scrollback and grid reflow behavior during resize
  - Prevent content at top from being incorrectly pushed to scrollback during resize
  - Use correct column width when pulling content from scrollback
  - Pull content back from scrollback when window widens
  - Push TOP content to scrollback while keeping BOTTOM visible on reflow (matches expected terminal behavior)
  - Preserve excess content in scrollback during reflow operations

## [0.11.0] - 2025-11-26

### Added
- **Full Terminal Reflow on Width Resize**: Both scrollback AND visible screen content now reflow when terminal width changes
  - **Scrollback Reflow**: Previously, changing terminal width would clear all scrollback to avoid panics from misaligned cell indexing. Now implements intelligent reflow similar to xterm and iTerm2
  - **Main Grid Reflow**: Visible screen content now also reflows instead of being clipped
    - **Width increase**: Unwraps previously soft-wrapped lines into longer lines
    - **Width decrease**: Re-wraps lines that no longer fit, preserving all content
  - Preserves all cell attributes (colors, bold, italic, etc.) during reflow
  - Handles wide characters (CJK, emoji) correctly at line boundaries
  - Properly manages circular buffer during scrollback reflow
  - Respects max_scrollback limits when reflow creates additional lines
  - Significant UX improvement for terminal resize operations

### Changed
- Height-only resize operations no longer trigger reflow (optimization)
- Scrollback buffer is now rebuilt (non-circular) after reflow for simpler indexing
- Main grid now extracts logical lines and re-wraps them on width change

## [0.10.0] - 2025-11-24

### Added
- **Emoji Sequence Preservation**: Complete support for complex emoji sequences and grapheme clusters
  - **Variation Selectors**: Preserves emoji vs text style presentation (U+FE0E, U+FE0F)
    - Example: ⚠ vs ⚠️ (warning sign in text vs emoji style)
  - **Skin Tone Modifiers**: Supports Fitzpatrick scale skin tones (U+1F3FB-U+1F3FF)
    - Example: 👋🏽 (waving hand with medium skin tone)
  - **Zero Width Joiners (ZWJ)**: Preserves multi-emoji sequences
    - Example: 👨‍👩‍👧‍👦 (family), 🏳️‍🌈 (rainbow flag)
  - **Regional Indicators**: Proper handling of flag emoji
    - Example: 🇺🇸 (US flag), 🇬🇧 (UK flag)
  - **Combining Characters**: Supports diacritics and other combining marks
    - Example: é (e + combining acute accent)
  - New `grapheme` module with comprehensive Unicode detection utilities
  - Enhanced `Cell` structure with `combining: Vec<char>` field for grapheme cluster storage
  - New methods: `Cell::get_grapheme()` and `Cell::from_grapheme()`
  - Python bindings now export full grapheme clusters through `get_line()` and `row_text()`

- **Web Terminal Frontend**: Modern Next.js-based web interface for the streaming server
  - Built with Next.js 16, React 19, TypeScript, and Tailwind CSS v4
  - **Mobile-Responsive Design**: Fully functional on phones and tablets
    - Responsive font sizing (4px mobile to 14px desktop)
    - Hideable header/footer to maximize terminal space
    - Touch support for mobile keyboard activation
    - Orientation change handling with automatic refit
    - Optimized scrollback (500 lines mobile, 1000 desktop)
    - Disabled cursor blink on mobile for battery savings
  - **Auto-Reconnect**: Exponential backoff (500ms to 5s max) with cancel button
  - Theme support with configurable color palettes
  - Nerd Font support for file/folder icons
  - WebGL renderer with DOM fallback
  - React 18 StrictMode compatible
  - Dev server binds to all interfaces (0.0.0.0) for mobile testing
  - New Makefile targets for web frontend development

- **Terminal Sequence Support**:
  - **CSI 3J**: Clear scrollback buffer command
  - Improved cursor positioning for snapshot exports

### Fixed
- **Graphics Scrollback**: Graphics now properly preserved when scrolling into scrollback buffer
  - Added `scroll_offset_rows` tracking for proper graphics rendering
  - Tall Sixel graphics preserved when bottom is still visible
  - Fixed premature scroll_offset during Sixel load
- **Sixel Scrollback**: Content now saved to scrollback during large Sixel scrolling operations
- **Kitty Graphics Protocol**: Fixed animation control parsing bugs
  - Support for both padded and unpadded base64 encoding
  - Corrected frame action handling for animations

### Changed
- **Breaking**: `Cell` struct no longer implements `Copy` trait (now `Clone` only)
  - Required for supporting variable-length grapheme clusters
  - All cell copy operations now require explicit `.clone()` calls
  - Performance impact is minimal due to efficient cloning

### Dependencies
- Added `unicode-segmentation = "1.12"` for grapheme cluster support

## [0.9.1] - 2025-11-23

### Fixed
- **Theme Rendering**: Fixed theme color palette application in Python bindings
  - Colors now properly use configured ANSI palette instead of hardcoded defaults
  - Affects `get_visible_lines()` method in `PtyTerminal`
  - Ensures theme colors are consistently rendered across all output methods
  - Resolves foreground and background colors using the active palette

### Added
- **Makefile**: Added `install-force` target for force uninstall and reinstall

## [0.9.0] - 2025-11-22

### Added
- **Graphics Protocol Support**: Comprehensive multi-protocol graphics implementation
  - **iTerm2 Inline Images** (OSC 1337): PNG, JPEG, GIF support with base64 encoding
  - **Kitty Graphics Protocol** (APC G): Advanced image placement with reuse and animations
  - **Sixel Graphics**: Enhanced with unique IDs and configurable cell dimensions
  - Unified `GraphicsStore` with scrollback support and memory limits
  - Animation support with frame composition and timing control
  - Graphics dropped event tracking for resource management

- **Pre-built Streaming Server Binaries**: Download ready-to-run binaries from GitHub Releases
  - Linux (x86_64, ARM64), macOS (Intel, Apple Silicon), Windows (x86_64)
  - No compilation needed - just download and run
  - Includes separate web frontend package (tar.gz/zip) for serving the terminal interface
  - Published to crates.io for Rust developers: `cargo install par-term-emu-core-rust --features streaming`

## [0.8.0] - 2025-11-19

### Fixed
- **Keyboard Protocol Reset**: Automatically reset Kitty Keyboard Protocol flags when exiting alternate screen buffer
  - Prevents TUI apps from leaving keyboard in bad state if they fail to disable protocol on exit
  - Clears both main and alternate keyboard flag stacks
  - Ensures clean terminal state after TUI app termination

## [0.7.0] - 2024-11-19

### Added
- **Buffer Controls**: Configurable limits for system resources
  - `set_max_notifications()` / `get_max_notifications()`: Limit OSC 9/777 notification backlog
  - `set_max_clipboard_sync_events()` / `get_max_clipboard_sync_events()`: Limit clipboard event history
  - `set_max_clipboard_event_bytes()` / `get_max_clipboard_event_bytes()`: Truncate large clipboard payloads
- **XDG Base Directory Compliance**: Shell integration now follows XDG standards
- **Improved Session Export**: Enhanced `export_asciicast()` and `export_json()` with explicit session parameters

### Changed
- **Shell Integration**: Migrated to XDG Base Directory specification for better standards compliance
- **Export APIs**: Session parameter now explicit in export methods for clearer API

### Documentation
- Comprehensive documentation for all new features and buffer controls
- Updated examples for new buffer control APIs

## [0.6.0] - 2024-11-15

### Added
- **Comprehensive Color Utilities API**: 18 new Python functions for color manipulation
  - Brightness and contrast: `perceived_brightness_rgb()`, `adjust_contrast_rgb()`
  - Basic adjustments: `lighten_rgb()`, `darken_rgb()`
  - WCAG accessibility: `color_luminance()`, `is_dark_color()`, `contrast_ratio()`, `meets_wcag_aa()`, `meets_wcag_aaa()`
  - Color mixing: `mix_colors()`, `complementary_color()`
  - Color space conversions: `rgb_to_hsl()`, `hsl_to_rgb()`, `rgb_to_hex()`, `hex_to_rgb()`, `rgb_to_ansi_256()`
  - Advanced adjustments: `adjust_saturation()`, `adjust_hue()`
- **iTerm2 Compatibility**: Matching NTSC brightness formula and contrast adjustment algorithms
- **Python Bindings**: All color utilities exposed via `par_term_emu_core_rust` module
- **Fast Native Implementation**: Rust-based for optimal performance

## [0.5.0] - 2024-11-10

### Added
- **Bold Brightening Support**: Configurable bold brightening for improved terminal compatibility
  - `set_bold_brightening()` method: Enable/disable bold text brightening for ANSI colors 0-7
  - iTerm2 Compatibility: Matches iTerm2's "Use Bright Bold" setting behavior
  - Automatic Color Conversion: Bold text with ANSI colors 0-7 automatically uses bright variants 8-15
  - Snapshot Integration: `create_snapshot()` automatically applies bold brightening when enabled

### Changed
- Enhanced `create_snapshot()` to automatically apply bold brightening when enabled

### Documentation
- New section in `docs/ADVANCED_FEATURES.md` with bold brightening examples

## [0.4.0] - 2024-11-01

### Added
- **Session Recording and Replay**: Record terminal sessions with timing information
  - Multiple event types: input, output, resize, custom markers
  - Export formats: asciicast v2 (asciinema) and JSON
  - Session metadata capture
  - Markers/bookmarks support
- **Terminal Notifications**: Advanced notification system
  - Multiple trigger types: Bell, Activity, Silence, Custom
  - Alert options: Desktop, Sound (with volume), Visual
  - Configurable settings per trigger type
  - Activity/silence detection
  - Event logging with timestamps
- **Enhanced Screenshot Support**:
  - Theme configuration options
  - Custom link and bold colors
  - Minimum contrast adjustment
- **Buffer Statistics**: Comprehensive terminal content analysis
  - `get_stats()`: Detailed terminal metrics
  - `count_non_whitespace_lines()`: Content line counting
  - `get_scrollback_usage()`: Scrollback buffer tracking

### Changed
- Improved screenshot configuration with theme settings
- Enhanced export functionality for better session capture

## [0.3.0] - 2024-10-20

### Added
- **Text Extraction Utilities**: Smart word/URL detection, selection boundaries
  - `get_word_at()`: Extract word at cursor with customizable word characters
  - `get_url_at()`: Detect and extract URLs
  - `select_word()`: Get word boundaries for double-click selection
  - `get_line_unwrapped()`: Get full logical line following wraps
  - `find_matching_bracket()`: Find matching brackets/parentheses
  - `select_semantic_region()`: Extract content within delimiters
- **Content Search**: Find text with case-sensitive/insensitive matching
  - `find_text()`: Find all occurrences
  - `find_next()`: Find next occurrence from position
- **Static Utilities**: Standalone text processing functions
  - `Terminal.strip_ansi()`: Remove ANSI codes
  - `Terminal.measure_text_width()`: Measure display width
  - `Terminal.parse_color()`: Parse color strings

## [0.2.0] - 2024-10-10

### Added
- **Screenshot Support**: Multiple format support
  - Formats: PNG, JPEG, BMP, SVG (vector), HTML
  - Embedded JetBrains Mono font
  - Programming ligatures support
  - Box drawing character rendering
  - Color emoji support with font fallback
  - Cursor rendering with multiple styles
  - Sixel graphics rendering
  - Minimum contrast adjustment
- **PTY Support**: Interactive shell sessions
  - Spawn commands and shells
  - Bidirectional I/O
  - Process management
  - Dynamic resizing with SIGWINCH
  - Environment control
  - Event loop integration
  - Context manager support
  - Cross-platform (Linux, macOS, Windows)

### Changed
- Improved Unicode handling for wide characters and emoji
- Enhanced grid rendering for box drawing characters

## [0.1.0] - 2024-10-01

### Added
- Initial stable release
- **Core VT Compatibility**: VT100/VT220/VT320/VT420/VT520 support
- **Rich Color Support**: 16 ANSI, 256-color palette, 24-bit RGB
- **Text Attributes**: Bold, italic, underline (multiple styles), strikethrough, blink, reverse, dim, hidden
- **Advanced Cursor Control**: Full VT100 cursor movement
- **Line/Character Editing**: VT220 insert/delete operations
- **Rectangle Operations**: VT420 fill/copy/erase/modify rectangular regions
- **Scrolling Regions**: DECSTBM support
- **Tab Stops**: Configurable tab stops
- **Terminal Modes**: Application cursor keys, origin mode, auto wrap, alternate screen
- **Mouse Support**: Multiple tracking modes and encodings
- **Modern Features**:
  - Alternate screen buffer
  - Bracketed paste mode
  - Focus tracking
  - OSC 8 hyperlinks
  - OSC 52 clipboard operations
  - OSC 9/777 notifications
  - Shell integration (OSC 133)
  - Sixel graphics
  - Kitty Keyboard Protocol
  - Tmux Control Protocol
- **Scrollback Buffer**: Configurable history
- **Terminal Resizing**: Dynamic size adjustment
- **Unicode Support**: Full Unicode including emoji and wide characters
- **Python Integration**: PyO3 bindings for Python 3.12+

[0.37.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.36.0...v0.37.0
[0.36.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.35.0...v0.36.0
[0.35.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.34.0...v0.35.0
[0.34.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.33.0...v0.34.0
[0.33.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.32.0...v0.33.0
[0.32.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.31.1...v0.32.0
[0.31.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.31.0...v0.31.1
[0.31.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.30.0...v0.31.0
[0.30.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.29.0...v0.30.0
[0.29.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.28.0...v0.29.0
[0.28.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.27.0...v0.28.0
[0.27.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.26.0...v0.27.0
[0.26.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.25.0...v0.26.0
[0.25.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.24.0...v0.25.0
[0.24.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.23.0...v0.24.0
[0.23.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.22.1...v0.23.0
[0.22.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.22.0...v0.22.1
[0.22.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.20.1...v0.21.0
[0.20.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.20.0...v0.20.1
[0.20.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.5...v0.20.0
[0.19.5]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.4...v0.19.5
[0.19.4]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.3...v0.19.4
[0.19.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.2...v0.19.3
[0.19.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.1...v0.19.2
[0.19.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.19.0...v0.19.1
[0.19.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.2...v0.19.0
[0.18.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.1...v0.18.2
[0.18.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.18.0...v0.18.1
[0.18.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.3...v0.17.0
[0.16.3]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.2...v0.16.3
[0.16.2]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.9.1...v0.10.0
[0.9.1]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/paulrobello/par-term-emu-core-rust/releases/tag/v0.1.0
