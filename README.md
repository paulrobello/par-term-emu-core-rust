# Par Term Emu Core Rust

[![PyPI](https://img.shields.io/pypi/v/par_term_emu_core_rust)](https://pypi.org/project/par_term_emu_core_rust/)
[![Crates.io](https://img.shields.io/crates/v/par-term-emu-core-rust)](https://crates.io/crates/par-term-emu-core-rust)
[![PyPI - Python Version](https://img.shields.io/pypi/pyversions/par_term_emu_core_rust.svg)](https://pypi.org/project/par_term_emu_core_rust/)
![Runs on Linux | MacOS | Windows](https://img.shields.io/badge/runs%20on-Linux%20%7C%20MacOS%20%7C%20Windows-blue)
![Arch x86-64 | ARM | AppleSilicon](https://img.shields.io/badge/arch-x86--64%20%7C%20ARM%20%7C%20AppleSilicon-blue)
![PyPI - Downloads](https://img.shields.io/pypi/dm/par_term_emu_core_rust)
![Crates.io Downloads](https://img.shields.io/crates/d/par-term-emu-core-rust)
![PyPI - License](https://img.shields.io/pypi/l/par_term_emu_core_rust)

A comprehensive terminal emulator library written in Rust with Python bindings for Python 3.12+. Provides VT100/VT220/VT320/VT420/VT520 compatibility with PTY support, matching iTerm2's feature set.

[!["Buy Me A Coffee"](https://www.buymeacoffee.com/assets/img/custom_images/orange_img.png)](https://buymeacoffee.com/probello3)

## What's New

> **Note:** For the complete release history, see [CHANGELOG.md](CHANGELOG.md).
> The entries below cover the most recent releases; the full history for older
> versions lives in CHANGELOG.md.

Version 0.50.0 is a **minor release with one breaking Python API removal**: the dead terminal multiplexing module (`PaneState`/`WindowLayout`/`SessionState` and the `Terminal` pane-state methods) is removed — it had zero callers, captured lossy text-only state, and its restore was a stub; the Rust `mux` feature gains the par-mux Phase 2 command set (split-window/select-pane/resize-pane/swap-pane with `%layout-change` broadcasts, and tmux-convention `capture-pane -S`/`-E` ranges), the Phase 4 protocol set (lifecycle broadcasts, tmux-contract `send-keys`, absolute `resize-pane -x/-y`, `%exit` on shutdown), the Phase 5 agent layer (herdr-compatible JSON hook reports on the control socket, `%agent-state-changed` broadcasts with hook/scrape provenance, the `list-agents` roster, and the scrape tier — fallback state for claude/codex/grok panes read from title and screen patterns, override files beside the state file), and Phase 6 agent-session resume (session identity persists in the state file, and a restarted daemon respawns agent panes through their resume invocations). See [What's New in 0.50.0](#whats-new-in-0500) below, and [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.50.0

Version 0.50.0 is a **minor release** whose headline is a **breaking Python API removal** plus the par-mux Phase 2–6 command, agent, and resume work on the Rust side.

- **Removed (breaking):** the terminal multiplexing module — `PaneState`, `WindowLayout`, `SessionState`, and the `Terminal` pane-state methods (`capture_pane_state` and friends) — is gone from the Rust API and the Python bindings. It had zero callers in the codebase, its capture was plain rendered text (`Vec<String>`: no colors, attributes, scrollback, or alternate screen), and `restore_pane_state` restored only size/title/cursor by its own comment. `replay_snapshot.rs`'s `TerminalSnapshot` already captures everything a real persistence format needs, and par-mux Phase 3's on-disk format started from that type; no consumer of the removed API existed to migrate.
- **Added — par-mux Phase 2 and Phase 4 command set (Rust `mux` feature):** pane commands over the binary `LayoutTree` — `split-window` (`-h`/`-v` orientation, `-p` percent for the new pane, reply carries the new pane id), `select-pane`, `resize-pane` (`-L`/`-R`/`-U`/`-D` with cell count, default 5), and `swap-pane` (`-t`/`-s`) — each broadcasting a `%layout-change` layout string to every connected client; `capture-pane -S`/`-E` now selects an exact line range under tmux's offset convention (`0` = first visible line, negative = history lines counted back from the screen top) instead of a scrollback tail count; and `MuxClient` keeps — and can `kill_spawned_daemon()` — the daemon child it spawns instead of orphaning it. Phase 4 adds lifecycle broadcasts (`%window-add`/`%window-close`/`%window-renamed`/`%window-pane-changed`/`%session-changed`) on every mutating dispatch; `send-keys` with tmux key names, `-l`, and `-H` (no implicit newline); `resize-pane -x/-y` absolute sizing with pane terminals that actually follow the layout geometry (split/resize/swap/kill and restart-restore all re-fit terminals now); `refresh-client -C WxH` window-size reports (latest wins) with a `%layout-change` broadcast; `%exit` pushed to every client on graceful shutdown; and bare `new-window` plus documented fixed list-reply shapes.
- **Added — Phase 5 agent layer and scrape tier (Rust `mux` feature):** herdr's agent-hook integrations port with an env-var rename — a control-socket line starting with `{` is a JSON report (`pane.report_agent` / `pane.report_agent_session`) answered in place and never registered for broadcasts; accepted reports record agent state in pane metadata, broadcast `%agent-state-changed <pane> <agent> <state> source=hook`, and are ordered by a monotonic `seq` (stale reports dropped, `unknown` never written); every pane is seeded with `PAR_MUX_PANE_ID`/`PAR_MUX_SOCKET`/`PAR_MUX_ENV=1` so ported scripts run inside it; `AgentPaneFactory` pre-tags agent panes for embedders and the resume path; and `list-agents` replies the roster — one `%N <agent> <state> <source>` line per state-carrying pane (plus the blocked reason as the rest of the line, when the agent reports one), stateless panes absent. `pane.report_agent_session` identifies a session by id OR transcript path — herdr's contract, and the shape the pi/omp extensions send (path-only) — and its optional `session_resume_argv` is stored verbatim as `agent_resume_argv` metadata, the hook-first resume invocation Phase 6's per-agent table falls back from. Panes whose agent reports no state hook (claude, codex, grok) get state from title/screen pattern rules bundled from herdr and shadowable by `<state-dir>/par-mux/agent-patterns/<agent>.toml`; a pane that ever accepted a hook state report is hook-authoritative for good, an unmatched scrape clears its own guess (never `idle`), and every surface carries `hook`/`scrape` provenance so a claim is never mistaken for a guess.
- **Added — Phase 6 agent-session resume (Rust `mux` feature):** agent session identity now persists in the daemon's save format (`PersistPane`'s optional `agent_session`; `FORMAT_VERSION` 2 — a v1 state file is quarantined and the server starts fresh rather than partially reading it). The resume invocation is never persisted — it is rebuilt at restore time from a per-agent table shipping with the binary (claude/codex/grok/pi/omp, each in its own CLI's argument shape), with the hook-reported `session_resume_argv` taking precedence when present. Restore wires that invocation into the respawn — every failure mode of the chain falls back structurally to the pane's original spawn command, so a missing table entry or an uninstalled agent binary just spawns the pane as it was. A live-daemon restart test proves the resume is real: the restarted daemon respawns the pane through the persisted identity's invocation, and the resumed process's own report (same session id, `session_start_source=resume`) lands in the second save. Agent *state* deliberately does not persist — a restored pane reports state anew or holds none, keeping the post-restart roster empty by design.

See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.49.0

Version 0.49.0 is a **minor release** focused on graphics placement fidelity, recording interoperability, and web-terminal rendering. **Added:** `export_asciicast_v3()` exports recordings in the asciicast v3 format — nested `term` header, relative per-event intervals, `"COLSxROWS"` resize strings — with a `g` graphics event per graphic that entered the store during the recording (live placements plus scrollback promotions) carrying protocol, geometry, position, and base64 RGBA pixels the text-only stream cannot convey. Parameterized XTPUSHCOLORS/XTPOPCOLORS slot forms: `CSI Pi # P` stores the current dynamic + ANSI palette into slot Pi (1–10) without pushing and `CSI Pi # Q` restores it without popping, with stores past the current depth growing the stack and padding intermediate slots with current-color snapshots. DECSDM sixel display mode (`CSI ? 80 h/l`): set mode paints sixel graphics at the home position without scrolling or moving the cursor (xterm/mlterm/WezTerm semantics), reset keeps the default cursor-relative scrolling placement; DECRQM reports mode 80, DECSTR/RIS restore the default. Kitty display placements (`a=T`/`a=p`) now advance the cursor to the first line below the image, spanning the placement's row count via the ordinary newline path — scroll region and scrollback promotion apply as for a multi-row text write; `C=1` suppresses the move (chafa and similar emitters), and virtual placements (`U=1`) never move the cursor. The web terminal frontend renders Sixel `DCS q` and iTerm2 `OSC 1337` inline images via `@xterm/addon-image` (Kitty pixel placements remain unsupported by xterm.js). No breaking changes for Python or Rust consumers.

## What's New in 0.48.0

Version 0.48.0 is a **minor release** focused on Kitty graphics protocol fidelity, driven by Herdr's image output. **Added:** placement geometry metadata — lowercase `x=`/`y=`/`w=`/`h=` are parsed as the source crop rectangle and preserved as new `source_x`/`source_y`/`source_width`/`source_height` fields on `ImagePlacement` (included in the JSON graphics export), while uppercase `X=`/`Y=` are the within-cell display offsets; previously lowercase `x=`/`y=` were misread as cell offsets, losing the crop entirely. Redisplaying an explicit nonzero `(i=, p=)` pair is now an upsert instead of a duplicate. **Fixed:** Kitty APCs process in stream order so interleaved cursor moves position each image slice (previously every slice stacked at one position); `d=` delete commands resolve after the full key=value list, so `d=` preceding `i=`/`p=` (Herdr's order) no longer silently no-ops; explicit `c=`/`r=` footprints govern row spans, `graphics_at_row`, and scroll/scrollback behavior instead of native pixel sizes; an omitted `c=`/`r=` axis is computed from the cropped source aspect ratio per the protocol; and retransmitting an image ID deletes its previous placements (including scrollback and animation state) as the spec requires, ending duplicate placements on session replay. **Changed — breaking for Rust `rlib` consumers only:** `TerminalGraphic::height_in_rows` now takes `(fallback_cell_width, fallback_cell_height)`; the method is not exposed to Python. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.47.0

Version 0.47.0 is a **minor release** focused on VT-sequence fidelity, security hardening, and Python ergonomics. **Added:** the XTPUSHCOLORS/XTPOPCOLORS/XTREPORTCOLORS color palette stack (`CSI # P/Q/R` — previously `CSI # P` misrouted to DCH and deleted characters); legacy alternate-screen modes 47, 1047 and 1048 (only 1049 was wired, so apps hardcoding the legacy modes got no screen switch); X10 mouse tracking (DECSET 9 — the mode existed but no DECSET arm ever set it); DECSACE stream-vs-rectangle extent for DECCARA/DECRARA; `OSC 1337;CurrentDir=` as an iTerm2 working-directory alias for OSC 7; `Terminal.diff_snapshots()` returning a `SnapshotDiff` (the class existed but no binding produced one); genuine Python type stubs (`py.typed` + generated `_native.pyi` — 69 classes, 1,335 methods, 342 properties); a criterion VTE throughput benchmark suite (`make bench`, committed baseline); and a vitest harness for the web frontend. **Security:** streaming-server HTTP CORS no longer falls back to `very_permissive` when no origin allowlist is configured and the `/sessions` endpoint gained the WS origin guard; the Kitty file-transmission `..` check is component-wise (no longer rejects `my..notes.png`); debug-log temp files are PID-suffixed with `0600` and `O_NOFOLLOW`. **Fixed:** asciicast v2 export timestamps were 1000× too small (ms misread as µs); both WebSocket server paths (tungstenite and axum) now complete the RFC 6455 closing handshake instead of dropping with a bare FIN (clients saw 1006); Kitty placement APCs now store their decoded graphics; `get_word_at`/`select_word` are display-column correct for CJK/emoji; `tests/test_streaming.py` passes again under the streaming build. **Changed — breaking for Rust `rlib` consumers only:** MSRV raised 1.90 → 1.98 (PyPI wheel users unaffected), dead APIs removed from the `rlib` surface (none were exposed to Python), streaming codec conversions are macro-generated (`ConnectedBuilder` replaces five partial constructors — old ones deprecated one release), and `src/streaming/server.rs` was decomposed with types re-exported at their old paths. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.46.0

Version 0.46.0 is a **minor release** focused on a headless build profile and a logging fix. **Added:** the `sim` cargo feature — a headless terminal profile that compiles only grid + terminal + screenshot (no PTY, Python, or streaming), for pure-Rust embedders that vendor the crate as a server-side screen model (e.g. par-hack's char-mode simulation) without spawning real processes. The real-PTY backend is now gated behind a new `pty_session` feature that makes `portable-pty` and `nix` optional; `python` (the `PyPtyTerminal` binding) and `streaming-bin` (the server binary) auto-enable `pty_session`, so the default build, the PyPI wheel, and `par-term-streamer` are behaviorally unchanged. Consume with `cargo build --no-default-features --features sim`. **Fixed:** the Rust debug log now writes to the system temp dir (`std::env::temp_dir()`) instead of a hardcoded `/tmp/...` — on macOS the Rust log previously landed in `/tmp` while the Python TUI's log went to the per-user temp dir; both now agree (Linux is unchanged). **Internal:** removed the gitnexus/graphify dev-tooling integrations. Non-breaking for Python and Rust consumers. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.45.0

Version 0.45.0 is a **minor release** focused on host-integrated window reporting, a TUI rendering fix, and a broad dependency refresh. **Added:** host-supplied window state for XTWINOPS reports — the headless core previously hardcoded `CSI 11 t` to always report non-iconified and `CSI 13 t` / `CSI 13 ; 2 t` to report position `(0, 0)`; GUI hosts (e.g. par-term) can now call `set_window_iconified(bool)` / `set_window_position(x, y)` (with `window_iconified()` / `window_position()` getters) so these reports reflect the real on-screen window. Defaults are unchanged when the host never calls the setters, and a negative host-supplied coordinate is clamped to 0 in the `CSI 13 t` reply (CSI parameters are unsigned). Exposed on both Python `Terminal` and `PtyTerminal`. **Changed:** MSRV raised 1.88 → 1.90 — **breaking for direct Rust `rlib` consumers only** (no dependency requires it; set to a recent stable for broader compatibility); PyPI (Python wheel) users are unaffected, and the Linux glibc floor stays at 2.17 (manylinux2014). **Fixed:** a generation-counter race in `PtySession` where the counter could advance *before* the grid was written, letting a renderer stamp its cell cache with the already-advanced generation over not-yet-updated grid content — when that read was the last of an output burst, the counter never advanced again and partial regions froze until the next keypress/resize (visible in full-screen editors/pagers that repaint via partial line edits: joe, vim `DL`/`IL`/`EL`). A second generation bump now runs after the grid write, while the pre-processing bump is retained to preserve the issue #60 liveness guarantee. **Dependencies:** Rust `cargo update` (5 minor + 39 patch, no majors), Python (`uv lock --upgrade`: pillow, maturin, pyright, pytest, ruff), and web frontend (`ncu`: patch bumps + **pako 2 → 3**, with a one-line import fix in `lib/protocol.ts`); TypeScript held at 6.0.3. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

## What's New in 0.44.0

Version 0.44.0 is a **minor release** focused on terminal-reporting completeness, security hardening, and concurrency. **Added:** OSC 99 Kitty desktop notifications (`OSC 99 ; <metadata> ; <payload> ST`) with multi-chunk accumulation, urgency, and actions — exposed to Python via the new `take_notifications_detailed()` method and `Notification` class (the existing `take_notifications()`/`drain_notifications()` tuple API is unchanged); XTGETTCAP (`DCS + q`) replying terminal capabilities (`TN`/`Co`/`RGB`/`Tc`); DECRQSS (`DCS $ q`) replying current SGR / cursor-style / scroll-margin settings; and XTWINOPS report ops `CSI 11 t` / `13 t` / `19 t` (window state / position / size-in-characters). **Fixed:** a DCS routing bug where `DCS +q` (XTGETTCAP) and `DCS $q` (DECRQSS) were misrouted into the Sixel parser because routing keyed only on the final `q` byte. **Security:** Kitty PNG and iTerm2 image decoding now enforce a shared `MAX_IMAGE_PIXELS` product cap (closing a decompression-bomb DoS reachable from any terminal/SSH bytes), and the streaming-server CLI hides `api_key` / `http_password` / `http_password_hash` values from `--help`. **Performance:** observer/trigger callback dispatch and PTY device-query reply writes moved outside the `Terminal` write lock (the reader thread previously held the exclusive guard across blocking PTY writes and Python re-entry, stalling every concurrent reader), screenshot glyph bitmaps are shared via `Arc<[u8]>` instead of per-character deep copies, and the per-row `String` allocation in flag-emoji detection is gone. **Refactored** (internal, no API change): the 4,004-line `python_bindings/types.rs` god-file was split into 12 cohesive submodules behind an unchanged facade, and the duplicated WS/WSS handshake logic was consolidated into a single shared callback factory. Non-breaking for Python and Rust consumers. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

Full history: [CHANGELOG.md](CHANGELOG.md) — every release from 0.9.0 through 0.43.0
is documented there (older README release notes were merged into the changelog).

## Features

### Core Terminal Emulation

- **VT100/VT220/VT320/VT420/VT520 Support** - Comprehensive terminal emulation matching iTerm2
- **Rich Color Support** - 16 ANSI colors, 256-color palette, 24-bit RGB (true color)
- **Text Attributes** - Bold, italic, underline (5 styles), strikethrough, blink, reverse, dim, hidden
- **Advanced Cursor Control** - Full VT cursor movement and positioning
- **Line/Character Editing** - VT220 insert/delete operations
- **Rectangle Operations** - VT420 fill/copy/erase/modify rectangular regions (DECFRA, DECCRA, etc.)
- **Scrolling Regions** - DECSTBM for restricted scrolling areas
- **Tab Stops** - Configurable tab stops (HTS, TBC, CHT, CBT)
- **Unicode Support** - Full Unicode including complex emoji sequences and grapheme clusters
  - Variation selectors (emoji vs text presentation)
  - Skin tone modifiers (Fitzpatrick scale U+1F3FB-U+1F3FF)
  - Zero Width Joiner (ZWJ) sequences for multi-emoji glyphs
  - Regional indicators for flag emoji
  - Combining characters and diacritical marks

### Modern Features

- **Alternate Screen Buffer** - Full support with automatic cleanup
- **Mouse Support** - Multiple tracking modes and encodings (X10, Normal, Button, Any, SGR, URXVT)
- **Bracketed Paste Mode** - Safe paste handling
- **Focus Tracking** - Focus in/out events
- **OSC 8 Hyperlinks** - Clickable URLs in terminal (full TUI support)
- **OSC 52 Clipboard** - Copy/paste over SSH without X11
- **OSC 9/777/99 Notifications** - Desktop-style alerts and notifications, including Kitty's OSC 99 protocol (chunked payloads, urgency levels, actions) via `take_notifications_detailed()`
- **Terminal Capability Queries** - XTGETTCAP (`DCS + q`) capability lookup and DECRQSS (`DCS $ q`) SGR/cursor-style/scroll-margin introspection
- **Expanded XTWINOPS Reporting** - `CSI 11/13/19 t` report window state, position, and screen size in characters
- **Shell Integration** - OSC 133 (iTerm2/VSCode compatible), OSC 1337 RemoteHost for remote host detection
- **Semantic Buffer Zones** - OSC 133 FinalTerm markers partition scrollback into prompt, command, and output zones
- **Command Output Capture** - Extract text from specific command execution blocks via `get_command_output()` and `get_command_outputs()`
- **Semantic Snapshot API** - Structured terminal state extraction via `get_semantic_snapshot()` for AI/LLM consumption, with configurable scope (visible, recent, full) and JSON output
- **Kitty Keyboard Protocol** - Progressive keyboard enhancement with auto-reset on alternate screen exit
- **Synchronized Updates (DEC 2026)** - Flicker-free rendering
- **Tmux Control Protocol** - Control mode integration support
- **Observer API** - Push-based event delivery with sync callbacks and async queues; convenience wrappers for common patterns (bell, title, CWD, command completion, zone changes)
- **General-purpose File Transfer** - OSC 1337 `File=` with `inline=0` for host-to-terminal downloads, `RequestUpload` for terminal-to-host uploads, with progress tracking and lifecycle events
- **Instant Replay** - Cell-level terminal snapshots with input-stream delta recording, size-based eviction, and timeline navigation via `SnapshotManager` and `ReplaySession`
- **C-Compatible FFI** - `#[repr(C)]` types (`SharedState`, `SharedCell`) and C API (`terminal_get_state`, `terminal_add_observer`) for embedding in C/C++ applications

### Graphics Support

- **Sixel Graphics** - DEC VT340 compatible bitmap graphics with half-block rendering
- **iTerm2 Inline Images** - OSC 1337 protocol for PNG, JPEG, GIF images
- **Kitty Graphics Protocol** - APC G protocol with image reuse, animations, zlib compression (`o=z`), and advanced placement
- **Unicode Placeholders** - Virtual placements insert U+10EEEE characters for inline image display
- **Unified Graphics Store** - Protocol-agnostic storage with scrollback support
- **Animation Support** - Frame-based animations with timing and composition control
- **Resource Management** - Configurable memory limits and graphics dropped tracking

### PTY Support

- **Interactive Shell Sessions** - Spawn and control shell processes
- **Bidirectional I/O** - Send input and receive output
- **Process Management** - Start, stop, and monitor child processes
- **Dynamic Resizing** - Resize with SIGWINCH signal
- **Environment Control** - Custom environment variables and working directory
- **Event Loop Integration** - Non-blocking update detection
- **Cross-Platform** - Linux, macOS, and Windows via portable-pty

### Terminal Streaming (WebSocket)

- **Standalone Server** - Pure Rust streaming server binary (no Python required)
- **Real-time Streaming** - Sub-100ms latency terminal streaming over WebSocket
- **Multiple Clients** - Support for concurrent viewers per session
- **Authentication** - Optional API key authentication (header or URL param)
- **Configurable Themes** - Multiple built-in color themes (iTerm2, Monokai, Dracula, Solarized)
- **Auto-resize** - Client-initiated terminal resizing with SIGWINCH support
- **Browser Compatible** - Works with any WebSocket client (xterm.js recommended)
- **Modern Web Frontend** - Next.js/React application with Tailwind CSS v4 and xterm.js

### Terminal Multiplexer (par-mux)

- **tmux Control Mode** - Sessions, windows, and split panes served over a local socket, attachable by tmux control-mode clients
- **Real-Time Push** - Pane output streams to clients as bytes arrive; no polling
- **Agent Awareness** - Panes report agent state over the same socket (JSON hook reports), with a scrape fallback for agents without hooks and an agent roster query
- **Session Resume** - Agent session identity persists across daemon restarts; agent panes respawn through their resume invocations
- **Crash-Safe State** - Atomic saves with quarantine of unreadable state files; a clean SIGTERM never loses the last window

### Screenshots and Export

- **Multiple Formats** - PNG, JPEG, BMP, SVG (vector), HTML
- **Embedded Font** - JetBrains Mono bundled - no installation required
- **Programming Ligatures** - =>, !=, >=, and other code ligatures
- **True Font Rendering** - High-quality antialiasing for raster formats
- **Color Emoji Support** - Full emoji rendering with automatic font fallback
- **Session Recording** - Record/replay sessions (asciicast v2, JSON)
- **Export Functions** - Plain text, ANSI styled, HTML export

### Macro Recording and Playback

- **YAML Format** - Human-readable macro storage format
- **Friendly Key Names** - Intuitive key combinations (`ctrl+shift+s`, `enter`, `f1`, etc.)
- **Keyboard Events** - Record and replay keyboard input with precise timing
- **Delays** - Control timing between events
- **Screenshot Triggers** - Trigger screenshots during playback
- **Playback Controls** - Play, pause, resume, stop, and speed control
- **Macro Library** - Store and manage multiple macros
- **Recording Conversion** - Convert terminal recording sessions to macros

### Utility Functions

- **Text Extraction** - Smart word/URL detection, selection boundaries, bracket matching
- **Content Search** - Find text with case-sensitive/insensitive matching
- **Buffer Statistics** - Memory usage, cell counts, graphics count and memory tracking
- **Color Utilities** - 18+ color manipulation functions (iTerm2-compatible)
  - NTSC brightness, contrast adjustment, WCAG accessibility checks
  - Color space conversions (RGB, HSL, Hex, ANSI 256)
  - Saturation/hue adjustment, color mixing

## Documentation

- **[API Reference](docs/API_REFERENCE.md)** - Complete Python API documentation
- **[VT Sequences](docs/VT_SEQUENCES.md)** - Comprehensive ANSI/VT sequence reference
- **[Advanced Features](docs/ADVANCED_FEATURES.md)** - Detailed feature guides
- **[Architecture](docs/ARCHITECTURE.md)** - Internal architecture details
- **[Security](docs/SECURITY.md)** - PTY security best practices
- **[Building](docs/BUILDING.md)** - Build instructions and requirements
- **[Configuration Reference](docs/CONFIG_REFERENCE.md)** - Configuration options
- **[Cross-Platform Notes](docs/CROSS_PLATFORM.md)** - Platform-specific information
- **[VT Technical Reference](docs/VT_TECHNICAL_REFERENCE.md)** - Detailed VT compatibility and implementation
- **[Fonts](docs/FONTS.md)** - Font configuration and rendering
- **[Macros](docs/MACROS.md)** - Macro recording and playback system
- **[Streaming](docs/STREAMING.md)** - WebSocket terminal streaming
- **[Multiplexer](docs/MUX.md)** - par-mux terminal multiplexer daemon
- **[Rust Usage](docs/RUST_USAGE.md)** - Using the library in pure Rust projects
- **[Observers](docs/OBSERVERS.md)** - Push-based event delivery (callbacks and asyncio queues)
- **[Instant Replay](docs/INSTANT_REPLAY.md)** - Cell-level snapshots and timeline navigation
- **[FFI Guide](docs/FFI_GUIDE.md)** - C-compatible embedding API for Swift/JNI/C/C++
- **[Graphics Testing](docs/GRAPHICS_TESTING.md)** - Testing graphics protocol implementations

## Installation

### From PyPI

```bash
uv add par-term-emu-core-rust
# or
pip install par-term-emu-core-rust
```

### From Source

Requires Rust 1.98+ and Python 3.12+:

```bash
# Install maturin (build tool)
uv tool install maturin

# Build and install
maturin develop --release
```

### Building a Wheel

```bash
maturin build --release
uv add --find-links target/wheels par-term-emu-core-rust
# or
pip install target/wheels/par_term_emu_core_rust-*.whl
```

### Using as a Rust Library

The library can be used in pure Rust projects without Python. Choose your feature combination:

| Use Case | Cargo.toml | What's Included |
|----------|------------|-----------------|
| **Rust Only** | `par-term-emu-core-rust = { version = "0.46", default-features = false, features = ["pty_session"] }` | Terminal, PTY, Macros |
| **Rust + Streaming** | `par-term-emu-core-rust = { version = "0.46", default-features = false, features = ["streaming", "pty_session"] }` | + WebSocket/HTTP server |
| **Python Only** | `par-term-emu-core-rust = "0.46"` | + Python bindings |
| **Everything** | `par-term-emu-core-rust = { version = "0.46", features = ["full"] }` | All features |

> **Note:** Since v0.46.0 the `pty_session` module (real PTY backend) is a separate feature. Omit it only for headless use without `PtySession`. See [docs/RUST_USAGE.md](docs/RUST_USAGE.md) for details.

**Download pre-built streaming server (recommended):**

Pre-built binaries and web frontend packages are available from [GitHub Releases](https://github.com/paulrobello/par-term-emu-core-rust/releases):

```bash
# Download binary (Linux example)
wget https://github.com/paulrobello/par-term-emu-core-rust/releases/latest/download/par-term-streamer-linux-x86_64
chmod +x par-term-streamer-linux-x86_64

# Download web frontend (asset named par-term-web-frontend-v<version>.tar.gz from
# https://github.com/paulrobello/par-term-emu-core-rust/releases/latest)
mkdir -p ./web_term
tar -xzf par-term-web-frontend-v*.tar.gz -C ./web_term

# Run
./par-term-streamer-linux-x86_64 --web-root ./web_term
```

Available binaries: Linux (x86_64, ARM64), macOS (Intel, Apple Silicon), Windows (x86_64)

**Or install from crates.io:**
```bash
cargo install par-term-emu-core-rust --features streaming-bin
```

**Or build from source:**
```bash
cargo build --bin par-term-streamer --no-default-features --features streaming-bin --release
./target/release/par-term-streamer --help
```

See [docs/RUST_USAGE.md](docs/RUST_USAGE.md) for detailed Rust API documentation and examples.

### Optional Components

#### Terminfo Installation

For optimal terminal compatibility, install the par-term terminfo definition:

```bash
# Install for current user
./terminfo/install.sh

# Or install system-wide
sudo ./terminfo/install.sh --system

# Then use
export TERM=par-term
export COLORTERM=truecolor
```

See [terminfo/README.md](terminfo/README.md) for details.

#### Shell Integration

Enhances terminal with semantic prompt markers, command status tracking, and smart selection:

```bash
cd shell_integration
./install.sh  # Auto-detects bash/zsh/fish
```

See [shell_integration/README.md](shell_integration/README.md) for details.

## Quick Start

### Basic Terminal Emulation

```python
from par_term_emu_core_rust import Terminal

# Create terminal
term = Terminal(80, 24)

# Process ANSI sequences
term.process_str("Hello, \x1b[31mWorld\x1b[0m!\n")
term.process_str("\x1b[1;32mBold green text\x1b[0m\n")

# Get content and cursor position
print(term.content())
col, row = term.cursor_position()
print(f"Cursor at: ({col}, {row})")
```

### PTY (Interactive Shell)

```python
from par_term_emu_core_rust import PtyTerminal
import time

# Create PTY terminal and spawn shell
with PtyTerminal(80, 24) as term:
    term.spawn_shell()

    # Send commands
    term.write_str("echo 'Hello from shell!'\n")
    time.sleep(0.2)

    # Get output
    print(term.content())

    # Resize terminal
    term.resize(100, 30)

    # Exit shell
    term.write_str("exit\n")
# Automatic cleanup
```

#### Environment Variables and Working Directory

Pass environment variables and working directory directly to `spawn_shell()` without modifying
the parent process environment. This is safe for multi-threaded applications (e.g., Tokio):

```python
from par_term_emu_core_rust import PtyTerminal

# Spawn with custom environment variables
with PtyTerminal(80, 24) as term:
    term.spawn_shell(env={"MY_VAR": "hello", "DEBUG": "1"})
    term.write_str("echo $MY_VAR\n")  # Outputs: hello

# Spawn with custom working directory
with PtyTerminal(80, 24) as term:
    term.spawn_shell(cwd="/tmp")
    term.write_str("pwd\n")  # Outputs: /tmp

# Combine both
with PtyTerminal(80, 24) as term:
    term.spawn_shell(env={"PROJECT": "myapp"}, cwd="/home/user/projects")
```

The `spawn()` method also accepts `env` and `cwd` parameters:

```python
term.spawn("/bin/bash", ["-c", "echo $MY_VAR"], env={"MY_VAR": "test"}, cwd="/tmp")
```

### Screenshots

```python
term = Terminal(80, 24)
term.process_str("\x1b[1;31mHello, World!\x1b[0m\n")

# Save screenshot
term.screenshot_to_file("output.png")
term.screenshot_to_file("output.svg", format="svg")  # Vector graphics!
term.screenshot_to_file("output.html", format="html")  # Styled HTML

# Custom configuration
term.screenshot_to_file(
    "output.png",
    font_size=16.0,
    padding=20,
    include_scrollback=True,
    minimum_contrast=0.5,  # iTerm2-compatible contrast adjustment
)
```

### Color Utilities

```python
from par_term_emu_core_rust import (
    perceived_brightness_rgb,
    adjust_contrast_rgb,
    contrast_ratio,
    meets_wcag_aa,
    rgb_to_hex,
    hex_to_rgb,
    mix_colors,
)

# iTerm2-compatible contrast adjustment
adjusted = adjust_contrast_rgb((64, 64, 64), (0, 0, 0), 0.5)

# WCAG accessibility checks
ratio = contrast_ratio((0, 0, 0), (255, 255, 255))
print(f"Contrast ratio: {ratio:.1f}:1")
print(f"Meets WCAG AA: {meets_wcag_aa((0, 0, 0), (255, 255, 255))}")

# Color conversions
hex_color = rgb_to_hex((255, 128, 64))  # "#FF8040"
rgb = hex_to_rgb("#FF8040")  # (255, 128, 64)
mixed = mix_colors((255, 0, 0), (0, 0, 255), 0.5)  # Purple
```

### Macro Recording and Playback

```python
from par_term_emu_core_rust import Macro, PtyTerminal
import time

# Create a macro manually
macro = Macro("git_status")
macro.set_description("Check git status and show branch")
macro.add_key("g")
macro.add_key("i")
macro.add_key("t")
macro.add_key("space")
macro.add_key("s")
macro.add_key("t")
macro.add_key("a")
macro.add_key("t")
macro.add_key("u")
macro.add_key("s")
macro.add_key("enter")
macro.add_delay(500)  # Wait 500ms
macro.add_screenshot("git_status.png")  # Trigger screenshot

# Save to YAML
macro.save_yaml("git_status.yaml")

# Load and play back
term = PtyTerminal(80, 24)
term.spawn_shell()

# Load macro from file
loaded_macro = Macro.load_yaml("git_status.yaml")
term.load_macro("git_check", loaded_macro)

# Play the macro
term.play_macro("git_check", speed=1.0)  # Normal speed

# Tick to execute macro events
while term.is_macro_playing():
    if term.tick_macro():  # Returns True if event was processed
        time.sleep(0.01)  # Small delay for visual effect

    # Check for screenshot triggers
    triggers = term.get_macro_screenshot_triggers()
    for label in triggers:
        term.screenshot_to_file(label)

# Convert a recording to a macro
term.start_recording("test session")
term.write_str("ls -la\n")
time.sleep(0.5)
session = term.stop_recording()

# Convert and save
macro = term.recording_to_macro(session, "ls_command")
macro.save_yaml("ls_command.yaml")
```

## Examples

See the `examples/` directory for comprehensive examples:

### Basic Examples
- `basic_usage_improved.py` - Enhanced basic usage
- `colors_demo.py` - Color support
- `cursor_movement.py` - Cursor control
- `text_attributes.py` - Text styling
- `unicode_emoji.py` - Unicode/emoji support
- `scrollback_demo.py` - Scrollback buffer usage

### Advanced Features
- `alt_screen.py` - Alternate screen buffer
- `mouse_tracking.py` - Mouse events
- `bracketed_paste.py` - Bracketed paste
- `synchronized_updates.py` - Flicker-free rendering
- `shell_integration.py` - OSC 133 integration
- `test_osc52_clipboard.py` - SSH clipboard
- `test_kitty_keyboard.py` - Kitty keyboard protocol
- `hyperlink_demo.py` - Clickable URLs
- `notifications.py` - Desktop notifications
- `rectangle_operations.py` - VT420 rectangle ops

### Graphics and Export
- `display_image_sixel.py` - Sixel graphics
- `test_sixel_simple.py` - Simple sixel examples
- `test_sixel_display.py` - Advanced sixel display
- `screenshot_demo.py` - Screenshot features
- `feature_showcase.py` - Comprehensive TUI showcase

### PTY Examples
- `pty_basic.py` - Basic PTY usage
- `pty_shell.py` - Interactive shells
- `pty_resize.py` - Dynamic resizing
- `pty_event_loop.py` - Event loop integration
- `pty_mouse_events.py` - Mouse in PTY
- `pty_custom_env.py` - Custom environment variables
- `pty_multiple.py` - Multiple PTY sessions
- `pty_with_par_term.py` - Integration with par-term

### Terminal Streaming
- `streaming_demo.py` - Python WebSocket streaming server
- `streaming_client.html` - Browser-based terminal client

### Macros and Automation
- `demo.yaml` - Example macro definition

**Standalone Rust Server:**
```bash
# Build and run (default: ws://127.0.0.1:8099)
make streamer-run

# Run with authentication
make streamer-run-auth

# Or use cargo directly
cargo build --bin par-term-streamer --no-default-features --features streaming-bin --release
./target/release/par-term-streamer --port 8099 --theme dracula

# With authentication
./target/release/par-term-streamer --api-key my-secret --theme monokai

# With system resource stats (CPU, memory, disk, network)
./target/release/par-term-streamer --enable-system-stats --system-stats-interval 5

# Install globally
make streamer-install
par-term-streamer --help
```

**Available Themes:** `iterm2-dark`, `monokai`, `dracula`, `solarized-dark`

### Web Terminal Frontend

**Using Pre-built Package (Recommended):**

Download the pre-built static web frontend from [GitHub Releases](https://github.com/paulrobello/par-term-emu-core-rust/releases):

```bash
# Download and extract (asset named par-term-web-frontend-v<version>.tar.gz from
# https://github.com/paulrobello/par-term-emu-core-rust/releases/latest)
mkdir -p ./web_term
tar -xzf par-term-web-frontend-v*.tar.gz -C ./web_term

# Run streamer with web frontend
par-term-streamer --web-root ./web_term
# Open browser to http://localhost:8099
```

See [web-terminal-frontend/README.md](web-terminal-frontend/README.md) for detailed usage instructions.

**Building from Source:**

A modern Next.js-based web terminal frontend source is in `web-terminal-frontend/`:

```bash
cd web-terminal-frontend

# Install dependencies
npm install

# Development server (runs on port 8030)
npm run dev

# Build for production (outputs to out/)
npm run build

# Copy to web_term for serving
cp -r out/* ../web_term/
```

**Features:**
- Modern UI with Tailwind CSS v4
- xterm.js terminal emulator
- WebSocket connection to streaming server
- Theme selection and synchronization
- Responsive design
- Terminal resize support
- **Customizable UI theme** - Edit `theme.css` after build (no rebuild required)

See [web-terminal-frontend/README.md](web-terminal-frontend/README.md) for detailed setup and configuration.

## TUI Demo Application

A full-featured TUI (Text User Interface) application is available in the sister project [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust).

![TUI Demo Application](https://raw.githubusercontent.com/paulrobello/par-term-emu-tui-rust/refs/heads/main/Screenshot.png)

**Installation:** `uv add par-term-emu-tui-rust` or `pip install par-term-emu-tui-rust`

**GitHub:** [https://github.com/paulrobello/par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust)

## Technology

- **Rust** (1.98+) - Core library implementation
- **Python** (3.12+) - Python bindings
- **PyO3** - Zero-cost Python/Rust bindings
- **VTE** - ANSI sequence parsing
- **portable-pty** - Cross-platform PTY support

## Running Tests

```bash
# Run Rust tests
cargo test

# Run Python tests
uv sync  # Install dependencies including pytest
pytest tests/
```

## Performance

- Zero-copy operations where possible
- Efficient grid representation
- Fast ANSI parsing with VTE crate
- Minimal Python/Rust boundary crossings

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for implementation details.

## Security

When using PTY functionality, follow security best practices to prevent command injection and other vulnerabilities.

See [docs/SECURITY.md](docs/SECURITY.md) for comprehensive security guidelines.

## Contributing

Contributions are welcome! Please submit issues or pull requests on GitHub.

### Development Setup

```bash
git clone https://github.com/paulrobello/par-term-emu-core-rust.git
cd par-term-emu-core-rust
make setup-venv  # Create virtual environment
make pre-commit-install  # Install pre-commit hooks (recommended)
make dev  # Build library
make checkall  # Run all quality checks
```

### Code Quality

All contributions must pass:
- Rust formatting (`cargo fmt`)
- Rust linting (`cargo clippy`)
- Python formatting (`make fmt-python`)
- Python linting (`make lint-python`)
- Type checking (`pyright`)
- Tests (`make test-python`)

**TIP:** Use `make pre-commit-install` to automate all checks on every commit!

See [CLAUDE.md](CLAUDE.md) for detailed development instructions.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

Paul Robello - probello@gmail.com

## Links

- **PyPI:** [https://pypi.org/project/par-term-emu-core-rust/](https://pypi.org/project/par-term-emu-core-rust/)
- **Crates.io:** [https://crates.io/crates/par-term-emu-core-rust](https://crates.io/crates/par-term-emu-core-rust)
- **GitHub:** [https://github.com/paulrobello/par-term-emu-core-rust](https://github.com/paulrobello/par-term-emu-core-rust)
- **TUI Application:** [https://github.com/paulrobello/par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust)
- **Documentation:** See [docs/](docs/) directory
- **Examples:** See [examples/](examples/) directory
