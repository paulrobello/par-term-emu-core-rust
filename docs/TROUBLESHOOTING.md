# Troubleshooting

One entry point for the most common build, test, streaming, and par-mux failures. Each entry gives the fix, with the cause where the source guide states it, and links to the guide that covers the area in depth.

## Table of Contents

- [Build and Install](#build-and-install)
  - [cargo build fails at the link stage](#cargo-build-fails-at-the-link-stage)
  - [cannot find -lpython3.x](#cannot-find--lpython3x)
  - [uv: command not found](#uv-command-not-found)
  - [no preset version for pyo3](#no-preset-version-for-pyo3)
- [Tests](#tests)
  - [cargo test fails to link or finds no tests](#cargo-test-fails-to-link-or-finds-no-tests)
- [Streaming Server](#streaming-server)
  - [WebSocket connection fails](#websocket-connection-fails)
  - [Client connects but shows no output](#client-connects-but-shows-no-output)
  - [Keystrokes have no effect](#keystrokes-have-no-effect)
- [par-mux Multiplexer](#par-mux-multiplexer)
  - [no daemon running on path](#no-daemon-running-on-path)
  - [Clients see old behavior after a rebuild](#clients-see-old-behavior-after-a-rebuild)
  - [refusing to start a nested daemon](#refusing-to-start-a-nested-daemon)
  - [another server owns path](#another-server-owns-path)
  - [this pane has hook-only access](#this-pane-has-hook-only-access)
- [Attach client](#attach-client)
  - [multiple par-mux servers are running](#multiple-par-mux-servers-are-running)
  - [unknown --mode](#unknown---mode)
  - [Render mode shows the wrong background](#render-mode-shows-the-wrong-background)
  - [Pastes arrive wrapped in 200~ and 201~](#pastes-arrive-wrapped-in-200-and-201)
  - [par-mux attach starts a daemon instead of attaching](#par-mux-attach-starts-a-daemon-instead-of-attaching)
  - [Cannot attach to a daemon named attach](#cannot-attach-to-a-daemon-named-attach)
  - [Terminal stays in the alternate screen after a crash](#terminal-stays-in-the-alternate-screen-after-a-crash)
- [Related Documentation](#related-documentation)

## Build and Install

### cargo build fails at the link stage

**Symptom:** a bare `cargo build` fails while linking the library.

**Cause:** the default `python` feature enables PyO3's `extension-module`, which produces a Python extension that cannot link as a normal Rust artifact.

**Fix:** build the Python module through maturin:

```bash
make dev
```

Use `make streamer-run` for the streaming server binary. A standalone `par-mux` binary builds from its workspace member with `cargo build -p par-mux --bin par-mux --features mux-bin`. See [BUILDING.md](BUILDING.md).

### cannot find -lpython3.x

**Cause:** the Python development headers are missing.

**Fix:** install them (`python3-dev` on Debian/Ubuntu, `python3-devel` on Fedora/RHEL, a Homebrew Python on macOS). Exact commands are in [BUILDING.md Troubleshooting](BUILDING.md#troubleshooting).

### uv: command not found

**Fix:** install uv, then rerun `make setup-venv`:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

### no preset version for pyo3

**Cause:** the active interpreter is older than the supported Python 3.12+.

**Fix:** check `python --version` and point the virtual environment at a supported interpreter.

## Tests

### cargo test fails to link or finds no tests

**Cause:** plain `cargo test` inherits the `extension-module` feature, which cannot link a test binary. Feature-gated modules also compile out without their feature, so a filter that targets them matches zero tests. Terminal-core tests (terminal, grid, pty_session, graphics, screenshot, …) live in the `par-term-emu-core` workspace member, so a root `cargo test` filter matches zero of them.

**Fix:** use the Makefile targets, or pass the test feature set explicitly:

```bash
make test-rust
cargo test -p par-term-emu-core [--features pty_session,screenshot,serde] test_name   # core tests
cargo test --lib --no-default-features --features pyo3/auto-initialize test_name
cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming test_name
cargo test --lib --no-default-features --features pyo3/auto-initialize,ffi ffi
```

See [BUILDING.md Running Tests](BUILDING.md#running-tests).

## Streaming Server

### WebSocket connection fails

**Fix:**

- Confirm the server is listening: `lsof -i :8099` (8099 is the default port).
- Check that firewall rules allow the port.
- Match the scheme to the server: `ws://` for plain HTTP, `wss://` when TLS is enabled.

### Client connects but shows no output

**Fix:**

- Check the browser console for JavaScript errors.
- Confirm the PTY session is still running (`pty_terminal.is_running()`).
- Send a known string with `server.send_output("test\r\n")` to separate a transport problem from a PTY problem.

### Keystrokes have no effect

**Fix:** read the session's `dropped_messages` counter from `curl http://localhost:8099/sessions` before and after typing with a client connected.

- **Counter does not move during macro playback:** expected. `--macro-file` serves every viewer read-only.
- **Counter rises with no PTY writer:** a Rust embedder must call `StreamingServer::set_pty_writer` before clients can type, and a closed session never regains its writer. Reconnect to a live session.
- **Counter rises while you type:** the rate limit or the input queue is dropping input. Raise `--input-rate-limit`, or check that the child process reads stdin.

Every drop cause is listed in [STREAMING.md Input Drops](STREAMING.md#input-drops). Rendering, performance, and graphics issues are covered in [STREAMING.md Troubleshooting](STREAMING.md#troubleshooting).

## par-mux Multiplexer

### no daemon running on path

**Cause:** client mode (`par-mux --cmd …`) never starts a daemon, so it exits 1 when nothing owns the socket. Without `--socket` or a name it resolves `$PAR_MUX_SOCKET` before the default socket.

**Fix:** start the daemon (`par-mux` or `par-mux <name>`), or pass the socket the daemon is serving with `--socket <path>`. `par-mux attach` with no target lists the live servers when more than one is running. See [MUX.md Client Mode](MUX.md#client-mode).

### Clients see old behavior after a rebuild

**Cause:** the daemon outlives its clients, so an old daemon keeps serving old code. The `version` command reports the daemon's build stamp for comparison.

**Fix:** restart the daemon on the same socket. The restart restores the saved state. Pass the daemon's name when it is not the default one:

```bash
par-mux --restart
par-mux work --restart
```

When the restarted daemon fails to start, its stderr is in `<state file>.log` beside the state file. See [MUX.md Command Line](MUX.md#command-line).

### refusing to start a nested daemon

**Cause:** serve mode refuses to start inside a par-mux pane, where `PAR_MUX_ENV` is set.

**Fix:** use `--cmd`, `--stop`, or `--restart` from inside a pane to operate the outer daemon. To start a nested daemon on purpose, set `PAR_MUX_ALLOW_NESTED=1`. See [MUX.md Nested daemons](MUX.md#nested-daemons).

### another server owns path

**Cause:** a live daemon already serves that socket path. A stale remnant (a dead socket file left by SIGKILL or a crash) is reclaimed automatically and does not produce this error.

**Fix:** connect to the running daemon with `--cmd` or `attach`, choose another name or `--socket`, or stop the running daemon with `par-mux [<name>] --stop` (or `par-mux --socket <path> --stop`).

### this pane has hook-only access

**Cause:** the daemon runs with `--pane-endpoints`, so `$PAR_MUX_SOCKET` inside a pane names a hook-only endpoint that refuses control commands.

**Fix:** start the daemon with `--expose-control-socket`, or pass the control socket with `--socket`. See [MUX.md Pane endpoints](MUX.md#pane-endpoints-opt-in).

## Attach client

These entries cover `par-mux attach`. The full reference is [MUX.md Attaching from a terminal](MUX.md#attaching-from-a-terminal).

### multiple par-mux servers are running

**Symptom:** `par-mux attach` exits 1 with `multiple par-mux servers are running — attach to one with par-mux attach <name|path>` and a list of the live servers.

**Cause:** with no NAME and no `--socket`, attach refuses to guess when more than one daemon is live. With exactly one live daemon it attaches directly.

**Fix:** pick one from the list: `par-mux attach <name>` or `par-mux attach --socket <path>`.

### unknown --mode

**Symptom:** `par-mux attach` exits 2 with `unknown --mode "…" — valid modes: render, passthrough`.

**Cause:** the `--mode` flag only accepts `render` (the default) and `passthrough`. A misspelled flag fails rather than running the wrong mode. An unknown `[client] mode` in the config file is different: it prints `unknown [client] mode "…" — using render` and continues in render mode.

**Fix:** pass `--mode render` or `--mode passthrough`, or correct `[client] mode` in the config file.

### Render mode shows the wrong background

**Symptom:** in render mode, empty cells (rows below the layout, short lines) use the terminal's default background instead of the theme you expect.

**Cause:** the client asks the host for its background with an OSC 11 probe (`ESC ] 11 ; ? ST`, 150 ms deadline) at startup. When the host does not answer in time, as on Windows consoles and terminals that do not implement OSC 11, the fill stays at the terminal default.

**Fix:** use a host terminal that answers OSC 11, such as Ghostty, iTerm2, or xterm. Over a slow SSH link, reattach once the link is idle so the reply arrives within the deadline.

### Pastes arrive wrapped in 200~ and 201~

**Symptom:** after a client or pane app crashes, pasting into the host shell inserts `^[[200~` before the text and `^[[201~` after it.

**Cause:** an app turned on bracketed-paste mode (`ESC [ ? 2004 h`) and exited without turning it off, so the host keeps wrapping pastes. Render mode handles these markers itself (the paste body goes to the focused pane verbatim), so the leftovers only show in the host shell.

**Fix:** turn the mode off in the host terminal:

```bash
printf '\e[?2004l'
```

`reset` also clears it.

### par-mux attach starts a daemon instead of attaching

**Symptom:** `par-mux attach` serves a new daemon named `attach`, or rejects `-t`, `--mode`, or `--prefix` as unexpected arguments. `par-mux --help` lists no `attach` command.

**Cause:** the binary was built without the `attach` cargo feature, so `attach` is parsed as a daemon NAME. The published 0.58.0 crate has no `attach` feature.

**Fix:** rebuild with the feature (from a source checkout until 0.58.1 is published):

```bash
cargo install --path crates/par-mux --features mux-bin,attach --locked
```

The release archives on GitHub are built with `attach`. See [MUX.md](MUX.md#standalone-binaries-github-releases).

### Cannot attach to a daemon named attach

**Cause:** the `attach` subcommand name shadows the positional NAME form, so `par-mux attach attach` cannot address a daemon literally named `attach`.

**Fix:** pass that daemon's socket path, which it prints at startup: `par-mux attach --socket <path>`.

### Terminal stays in the alternate screen after a crash

**Symptom:** after a render-mode client exits abnormally (a panic, or a config error after the screen was set up), the host terminal stays in the alternate screen with mouse reporting on, so the scrollback is hidden and clicks print escape codes.

**Cause:** render mode restores the alternate screen and mouse capture after its session ends normally. An abnormal exit skips that restore.

**Fix:** run `reset` in the host terminal.

## Related Documentation

- [BUILDING.md](BUILDING.md) - Build prerequisites, feature flags, and test commands
- [STREAMING.md](STREAMING.md) - Streaming protocol, server options, and full troubleshooting
- [MUX.md](MUX.md) - par-mux daemon operations and command reference
- [CROSS_PLATFORM.md](CROSS_PLATFORM.md) - Platform-specific PTY behavior
- [SECURITY.md](SECURITY.md) - Resource limits and PTY security
