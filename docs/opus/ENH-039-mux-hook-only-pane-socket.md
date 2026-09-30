# ENH-039: Least-privilege pane endpoint — a per-pane hook-only socket, so a pane process cannot drive other panes or kill the server

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-039]`.
> Sequencing:
> - After **SEC-127**. The new listener reuses the bounded line read, and must not ship with the unbounded one.
> - Batch with or after **QA-199** (the `handle_client` extraction), because the pane endpoint shares `read_control_line`.
> - Cross-repo: par-term's hook shims (`~/Repos/par-term/mux_hooks/*.sh`, `*.ps1`), its `mux_extensions/{pi,omp}` agents, and the user's `par-mux` skill are named in step 6. File a par-term board item after merge.
>
> **Decision gate (D-code convention, like AUDIT.md's D1–D6).** This card has two halves:
> - **Non-gated half: implement it.** The pane endpoint, its pane binding, a daemon flag `--pane-endpoints` / factory field `pane_endpoints: bool` that selects it, and docs. **The default stays `false`**, so `PAR_MUX_SOCKET` keeps naming the full control socket exactly as today.
> - **Default flip: declined by the user (2026-09-29). Never implement it.** The default stays `false` permanently. Pane-to-pane control through `$PAR_MUX_SOCKET` is a core feature: an agent in one pane must be able to spawn and drive agents in other panes, and the user's `par-mux` skill depends on the in-pane `par-mux --cmd` fallback. SECURITY.md documents the default as intended, and `--pane-endpoints` as opt-in isolation for users who want it.
>
> Every Verify bullet below uses the explicit flag, so it passes whichever way the gate is decided.

**Priority**: medium · **Estimate**: L

## Goal

Every process in every pane inherits `$PAR_MUX_SOCKET`, the daemon's full control socket. Any of them, including an AI agent steered by a prompt injection or a compromised build script, can therefore:
- `capture-pane` any other pane, reading its secrets or output;
- `send-keys` into any other pane, typing commands into a privileged shell;
- `respawn-pane -k` or `kill-pane` other panes;
- `kill-server`.

Most in-pane consumers need only the four hook methods (`pane.report_agent`, `pane.report_agent_session`, `pane.report_agent_telemetry`, `pane.release_agent`) for their own pane. Give each pane an endpoint that accepts exactly those methods, bound to that pane's id, and, when enabled, make it what `PAR_MUX_SOCKET` names inside a pane.

## Threat model

- **In scope: misuse through the inherited environment.**
  - A process that learns the socket from `$PAR_MUX_SOCKET` and uses it as intended, but for another pane or for server control.
  - Typical case: an agent's tool call running `"$PAR_MUX_BIN" --cmd "send-keys -t %0 …"` against a pane it was never meant to see.
  - Least privilege at the endpoint that the environment hands out closes this.
- **Out of scope: a same-uid attacker who goes looking.**
  - The full control socket sits at a deterministic per-user path (`default_socket_path`, `src/mux/ipc.rs:514-527`), and its only authorization is the peer euid check at accept (`accept_connection`, `ipc.rs:94-113`; `peer_is_current_user`, `:117-124`).
  - Any same-uid process can connect to it directly, ptrace the daemon, or read `~/.ssh`. This card is **not** a boundary against same-uid code and must not be documented as one.
  - SECURITY.md must say so in those words.
- **Not changed:** the 0600 socket, the directory check, the Windows owner-only DACL (`ipc.rs:64-68`), or the client's server-identity check.

## Current state

- **Env contract.**
  - `ShellPaneFactory::spawn_session` sets `PAR_MUX_PANE_ID`, then `PAR_MUX_SOCKET` and `PAR_MUX_ENV=1` when `socket_path` is set, plus `PAR_MUX_SESSION_ID`/`SESSION`/`WINDOW_ID`/`BIN` (`src/mux/pane.rs:549-565`).
  - Session env is applied first, so a client's `set-environment` cannot override the identity set (`:544-548`).
  - Documented at `docs/MUX.md:249-261`.
- **The CLI depends on it.** A `par-mux --cmd …` typed inside a pane with no `--socket` falls back to `$PAR_MUX_SOCKET` (`src/bin/par_mux/main.rs:163-171`, `resolve_socket_path` at `ipc.rs:536`; MUX.md:92,254). That is how the user's `par-mux` skill drives the daemon from an agent pane ("spawn a dev-server pane", "send keys", "capture output"). This is why the default flip is gated.
- **Hook traffic shares the control socket.**
  - `handle_client` classifies each line: a `{…}` line is a hook report answered in place, never registered for broadcasts; anything else is a control command (`match parse_line(&line)` at `src/mux/server.rs:621-630`, `hooks::handle_report` at `src/mux/hooks.rs:44-64`).
  - A hook report names its pane in `params.pane_id`, and nothing checks that the sender *is* that pane. Any process can report or release on behalf of any pane.
- **Where sockets live.**
  - `default_socket_path` puts the control socket in `$XDG_RUNTIME_DIR`, else a per-UID directory under the temp dir (`uid_socket_dir`), as `par-mux-<name>.sock`.
  - `prepare_socket_path` (`ipc.rs:459-497`) guards that directory, creates it, and reclaims stale remnants.
  - Unix-socket paths are capped by `sun_path`: 104 bytes on macOS, 108 on Linux.
- **Consumers of `PAR_MUX_SOCKET`.**
  - par-term hook shims: `mux_hooks/par-mux-{claude,codex,grok}-session-hook.{sh,ps1}`, which connect with Python `socket.AF_UNIX`.
  - par-term agent extensions: `mux_extensions/pi/par-mux-agent-state.ts:14`, `mux_extensions/omp/par-mux-omp-agent-state.ts:14`.
  - This repo's test assets: `tests/assets/par-mux-agent-state.sh` (a herdr port whose header promises "Renames, and nothing else", exercised by `herdr_kimi_script_env_renamed_drives_a_live_daemon` at `tests/mux_hooks.rs:642`) and `par-mux-fake-agent.sh`.
  - All of them send only hook JSON, except the `--cmd` fallback.
- **The nested-daemon guard** keys on `PAR_MUX_ENV=1` (`main.rs:208-217`), which must keep working.

## Options for binding a connection to its pane

| Option | How the daemon knows the caller's pane | Cost |
|---|---|---|
| A. Per-pane token | `PAR_MUX_PANE_TOKEN` env var, sent as a JSON field on every report | Every shim changes (a new field). Tokens are readable from `/proc/<pid>/environ` by same-uid processes, which is acceptable under this threat model. Persisted panes need token regeneration on restore |
| B. Per-pane socket path | The listener *is* the pane: one socket per pane | One listener plus accept thread per pane, created at spawn and removed at kill. Shims are unchanged: same env var, same JSON |
| C. Peer PID ancestry | `peer_creds().pid()`, then walk parents via `ProcessTable` to a pane's child PID | A process-table snapshot per hook line. Fails for double-forked or re-parented hooks (daemons, `nohup`, systemd-run). Not portable to Windows named pipes |

**Choose B.**
- It is the only option where the shims need **zero changes**: the variable keeps its name and the JSON is unchanged. That preserves the herdr "renames and nothing else" port property.
- The connection's identity is structural, not a secret.
- Cost: one mostly-idle thread per pane. Accept threads block in `accept()` and hold no tree lock. On Windows, use one named pipe per pane with the same owner-only DACL.
- **Bound it.** Refuse to create a pane endpoint past a configurable cap (default 256, with a `/// cap:` annotation so it lands in the caps table). Past the cap, do not export `PAR_MUX_SOCKET` for that pane rather than falling back to the full socket.

## Implementation (non-gated half)

1. **Pane endpoint** (`src/mux/pane_endpoint.rs`, new).
   - **Location.** `PaneEndpoint::bind(control_socket: &Path, pane_id) -> io::Result<Self>` places the socket **beside the control socket, in the same guarded runtime directory**: `<dir>/par-mux-<name>.pane-<N>.sock`. It must not go under `--state-dir`: that is persistent data, and a long temp `--state-dir` would overrun `sun_path`.
   - Bind through `prepare_socket_path` + `bind_local_listener`, so the same directory guard, stale-remnant reclaim, 0600 mode and euid check apply.
   - **Length check.** If the path exceeds the platform `sun_path` limit (103 bytes usable on macOS, 107 on Linux), return an error. The factory then leaves `PAR_MUX_SOCKET` unset for that pane and logs once. It never falls back to the full socket.
   - One accept thread per endpoint. Each accepted connection runs a reduced loop:
     - reuse SEC-127/QA-199's bounded `read_control_line`;
     - accept only `{…}` hook lines and call `hooks::handle_report_for(pane_id, line, tree)`;
     - answer any non-JSON line with a JSON error `{"error":"hook-only endpoint"}` and close.
   - `Drop` removes the socket file.
   - **Stale cleanup.** At daemon start, after the control socket binds, remove any `par-mux-<name>.pane-*.sock` remnants in that directory. Drop does not run on a crash or SIGKILL, and they would accumulate otherwise. Only remove files that are sockets and that fail a connect, using `prepare_socket_path`'s stale test.
2. **Pane binding in hooks** (`src/mux/hooks.rs`). Add `handle_report_for(bound: PaneId, …)`, which:
   - rejects a report whose `params.pane_id` names a different pane, with the error "pane_id does not match this endpoint";
   - accepts a report that omits `pane_id` and fills in `bound`.

   `handle_report` (full socket) is unchanged, so embedders and par-term's own control connection still report for any pane.
3. **Lifecycle** (`src/mux/pane.rs`, `src/mux/tree.rs`).
   - When `pane_endpoints` is on, `MuxPane` owns an `Option<PaneEndpoint>`, created in the factory before spawn so the env var can point at it.
   - Dropped on kill, and on respawn (then recreated).
   - Created again on restore, because endpoints are per daemon run and never persisted.
   - Held panes keep theirs: a dead pane's agent can still be released.
4. **Env contract** (`pane.rs:549-565`), only when `pane_endpoints` is on.
   - `PAR_MUX_SOCKET` names the pane endpoint.
   - New `PAR_MUX_CONTROL_SOCKET` names the full socket. It is set only when the factory's `expose_control_socket: bool` is true, or for panes in a session whose env carries `PAR_MUX_CONTROL=1` (set via `new-session -e PAR_MUX_CONTROL=1` or `set-environment -t <session> PAR_MUX_CONTROL 1`; `split-window` has no `-e` flag). This reuses the per-session env map (`src/mux/command.rs:844-846`), which the factory reads before applying the identity set.
   - `PAR_MUX_ENV`, `PAR_MUX_PANE_ID`, `PAR_MUX_BIN` and the id variables are unchanged.
   - `set-environment` still cannot override `PAR_MUX_SOCKET`/`PAR_MUX_CONTROL_SOCKET` (identity set last, as today).
   - With `pane_endpoints` off (the default), the env is byte-identical to today.
5. **CLI** (`src/bin/par_mux/main.rs`).
   - Add the daemon flags `--pane-endpoints` and `--expose-control-socket`.
   - The `--cmd` fallback order becomes `--socket` > NAME > `$PAR_MUX_CONTROL_SOCKET` > `$PAR_MUX_SOCKET` > default.
   - When the resolved target is a pane endpoint, a control command fails with "this pane has hook-only access; start the daemon with --expose-control-socket or pass --socket". The endpoint's JSON error is mapped to that message.
6. **Migration notes** (docs; no consumer change is required in the non-gated half).
   - par-term hook shims and agent extensions: **no change**. Same variable, same JSON, now pane-bound when enabled.
   - The user's `par-mux` skill: document `--expose-control-socket` and `PAR_MUX_CONTROL_SOCKET` for agent-driven pane control. This is outside the repo, so mention it in the PR.
   - `tests/assets/*.sh`: unchanged.
7. **Docs.**
   - MUX.md "Agent Hook Reports" env table: new rows and the fixed-at-spawn note.
   - MUX.md "Socket and State Paths": the pane socket naming and the length fallback.
   - MUX.md "Command Line": the two flags.
   - SECURITY.md: a par-mux "In-pane privilege" subsection with the threat model above, verbatim scope limits included.
   - CONFIG_REFERENCE env table (DOC-115).
   - CHANGELOG `[Unreleased]` "Added" and "Security".
8. **Windows.** A per-pane named pipe, `\\.\pipe\par-mux-<stem>-pane-<N>`, with the same DACL. Validate with the VM playbook in CLAUDE.md: both `cargo check` feature sets, then the `mux::` filter.

## Gated half (report, do not implement)

Flip the default to `pane_endpoints = true` (daemon and `ShellPaneFactory`), plus a CHANGELOG **Changed** entry, a MUX.md env-contract rewrite and the par-mux skill update. It needs the user's approval, because it breaks the in-pane `--cmd` fallback for daemons started without `--expose-control-socket`.

## Files to touch

- `src/mux/pane_endpoint.rs` (new), `src/mux/mod.rs`
- `src/mux/hooks.rs` (`handle_report_for`)
- `src/mux/pane.rs` (endpoint ownership, env contract), `src/mux/tree.rs` (kill, respawn and restore lifecycle)
- `src/mux/ipc.rs` (pane socket path helper, `sun_path` length check, stale sweep, Windows pipe name)
- `src/mux/server.rs` (startup stale sweep)
- `src/bin/par_mux/main.rs` (`--pane-endpoints`, `--expose-control-socket`, fallback order, refusal message)
- `tests/mux_hooks.rs`, `tests/mux_cli.rs`
- `docs/MUX.md`, `docs/SECURITY.md`, `docs/CONFIG_REFERENCE.md`, `CHANGELOG.md`

## Verify

- A new integration test passes under `cargo test --no-default-features --features rust-only,mux,serde --test mux_hooks -- --test-threads=1`. It starts a daemon with `--pane-endpoints`, and a pane runs a script that connects to `$PAR_MUX_SOCKET` and sends `capture-pane -t %0`, `send-keys -t %0 x` and `kill-server`. Each is refused, and the daemon, pane `%0`'s screen and every pane stay unchanged.
- In the same file, with `--pane-endpoints`, a hook report sent from pane `%1` via `$PAR_MUX_SOCKET` with `"pane_id":"%0"` is rejected with "pane_id does not match this endpoint". The same report with `"pane_id":"%1"`, or with `pane_id` omitted, is accepted and appears in `list-agents` for `%1`.
- The existing `herdr_kimi_script_env_renamed_drives_a_live_daemon` test passes both with and without `--pane-endpoints` (parameterize it), and `git diff --stat tests/assets/` is empty.
- A daemon started without either new flag exports the same `PAR_MUX_*` set as HEAD (asserted by a test comparing the env of a pane with and without the flags), and `par-mux --cmd list-panes` typed in its pane succeeds. With `--pane-endpoints --expose-control-socket` it also succeeds via `PAR_MUX_CONTROL_SOCKET`. With `--pane-endpoints` alone it exits non-zero with the hook-only message. All three cases are covered in `tests/mux_cli.rs`.
- `cargo test --lib --no-default-features --features rust-only,mux,serde mux::` passes, including a unit test that a pane socket path over the `sun_path` limit leaves `PAR_MUX_SOCKET` unset, and a unit test that the startup sweep removes a stale `par-mux-<name>.pane-7.sock` remnant. `make checkall` is green, and on the Windows VM `cargo check --lib --tests --no-default-features --features rust-only,mux,serde` passes.

## Rollback

With the default off, disable by simply not passing `--pane-endpoints`. For a full revert, remove the module, flags and env branches. No persisted state references pane endpoints, because they are recreated each run.
