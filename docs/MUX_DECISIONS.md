# par-mux Design Decisions

One-line summaries of the design decisions that par-mux code comments cite by number (`D3.3`, `T4.C`, …). The full plan, with rationale and alternatives, lives in `par-mux.md` in the `par-agent-os` repository (see [par-mux.md](par-mux.md)). This file keeps the citations resolvable from this repository.

## Decisions

| Id | Decision | Cited in |
|----|----------|----------|
| D1 | `mux` is a cargo feature of this crate, not a separate crate; it is absent from the `default` and `sim` builds | `src/mux/scrape.rs`, `Cargo.toml`, `tests/mux_feature_isolation.rs` |
| D2 | The mux runtime types are its own (`MuxSession`, `MuxWindow`, `MuxPane`), separate from the `terminal::multiplexing` serialization schema; `src/streaming/mux_factory.rs` cites it for keeping the daemon's pane the only owner of its terminal | `src/streaming/mux_factory.rs` |
| D3 | Parity tier: implement what par-term's tmux client drives, not all of tmux (no copy-mode, buffer stack, or `#{…}` format strings) | `src/mux/dispatch/`, `src/mux/tree/` |
| D3.1 | Persistence serializes the replay-snapshot types through feature-gated serde derives (`serde` feature), not a DTO layer | `Cargo.toml` |
| D3.2 | The state file is a versioned envelope; an unreadable or unknown-version file is quarantined and the daemon starts fresh | `src/mux/persist.rs`, `src/bin/par_mux/main.rs` |
| D3.3 | Save after every mutating command and on clean shutdown, atomically (temp file, fsync, rename); losing unsaved content on SIGKILL is accepted. The original synchronous save later moved to an off-lock coalescing worker (ARC-003) | `src/mux/dispatch/`, `src/mux/command/`, `src/mux/persist.rs`, `src/mux/server/`, `src/bin/par_mux/main.rs`, [MUX.md](MUX.md#shutdown-semantics) |
| D3.4 | The state file lives in the platform state directory, keyed by socket name, owner-only; the socket itself stays in the runtime/temp directory | `src/mux/persist.rs`, `Cargo.toml`, `src/bin/par_mux/main.rs` |
| D3.5 | Restore rebuilds layout and content with fresh processes: spawn first, then restore the saved screen; id allocation resumes past saved ids | `src/mux/persist.rs`, `src/mux/server/`, `src/mux/ids.rs`, `src/mux/pane.rs` |
| D4 | Transport is a cross-platform local socket (Unix socket, Windows named pipe); the WebSocket streaming path is untouched | `src/mux/ipc.rs`, `Cargo.toml` |
| D5 | The server is a daemon that outlives its clients, and par-term auto-spawns it | `src/mux/dispatch/`, `Cargo.toml`, `src/bin/par_mux/main.rs` |
| D5.4 | Reattach resync: the daemon replays a pane's current screen on request (`refresh-client` without `-C`), reusing the terminal's snapshot export | `src/mux/dispatch/` |
| D6.2 | An agent's resume invocation is never persisted; it is built at restore time from a table shipped in the binary, with a hook-reported invocation taking precedence | `src/mux/agent_resume.rs` |
| D6.3 | Restore respawns agent panes through that invocation and fails safe: when none can be built, the pane spawns its original command | `src/mux/agent_resume.rs`, `src/mux/persist.rs` |
| T4.B | `send-keys` contract: key names, `-l` literal, `-H` hex, and no implicit trailing newline | `src/mux/command/` |
| T4.C | Sizing: pane terminals re-fit to layout geometry after every change; `resize-pane -x/-y`; `refresh-client -C` sizes the window to the smallest-attached-client minimum over the render clients displaying it (supersedes the original latest-report-wins decision) | `src/mux/command/`, `src/mux/dispatch/`, `src/mux/tree/` |
| T4.E | Queries have fixed reply shapes and no `-F` format strings; push notifications cover what `-F` polling was for | `src/mux/dispatch/` |
| R7 | par-mux owns agent state for the panes in its sessions and never reconciles another tool's roster; par-term consumes that state rather than becoming a second owner. Recorded in `REPORT.md` in the same repository; `src/streaming/mux_factory.rs` applies the same no-second-owner rule to pane terminals | `src/streaming/mux_factory.rs` |

Test files under `tests/mux_*.rs` also cite D3, D3.3, and D3.5 for the behavior they pin.
