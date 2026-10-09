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

> **Note:** This section covers unreleased work on `main` and the last three
> releases. The complete release history is in [CHANGELOG.md](CHANGELOG.md).

**Unreleased (main):** par-mux gains **workspaces — a first-class level above sessions** (daemon > workspace > session > window > pane): `new-workspace`/`list-workspaces`/`select-workspace`/`rename-workspace`/`kill-workspace` commands, workspace-targeted `new-session -t`/`list-sessions -t`, a workspace-aware `list-sessions` reply shape, the argument-less `%workspaces-changed` broadcast, and persistence format **v3** (pre-workspace state files are quarantined, not migrated — mux features are not yet public, so there is no compatibility path). The crate is now a Cargo workspace: the terminal core lives in `par-term-emu-core` and the multiplexer in `par-mux`, both re-exported so every `par_term_emu_core_rust::…` path is unchanged. **Breaking for `cargo install` users:** `cargo install par-term-emu-core-rust --bin par-mux` no longer works; install the daemon with `cargo install par-mux --features mux-bin,attach` instead once the `par-mux` crate is published (until then, `cargo install --path crates/par-mux --features mux-bin,attach --locked` from a checkout). Render-mode panes no longer lose a TUI's bottom row under the pane-border ring: the client declares its per-pane chrome (`refresh-client -I`) and the daemon sizes each PTY to the painted interior. **Breaking for Rust embedders:** `MuxCommand::RefreshClient`, `MuxWindow` and `ClientView` gain a `chrome` field and `MuxTree::set_client_view` a `chrome` argument. See [CHANGELOG.md](CHANGELOG.md) Unreleased and [docs/MUX.md](docs/MUX.md).
Version 0.58.1 (pending; until it is published, build from source as shown under [Installation](#installation)) ships the **par-mux `attach` client** behind the new `attach` cargo feature (off by default; `--features mux-bin,attach` builds the daemon with the subcommand): `par-mux attach` passthrough puts one pane fullscreen in any terminal (byte pump, `C-b` prefix router, status row), `--mode render` is a full ratatui TUI — per-pane layout-aware rendering with dividers and focus highlight, DECCKM-aware key re-encoding, pane-relative mouse forwarding, client-side wheel scrollback, a status bar with sessions/windows/agent-roster chips, and `prefix-[` keyboard scrollback. Supporting daemon work: `list-windows/-panes -t` targeted tree queries with a `targeted` capability token, a `-t`-less `refresh-client -C/-p` size report, and a `PtySession` reader fix so quiet child deaths advance the generation counter and a dead pty can no longer spin the reader. See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

Version 0.58.0 is a **minor release with breaking changes for C/Swift and Rust embedders**: the C FFI lands its **v4 ABI revision** — every export is `ptec_`-prefixed, `SharedCell` colors resolve through the live palette with `ptec_terminal_read_cell_grapheme` for full graphemes, and observers receive structured `on_event_v2` events — plus **v5 scroll-aware damage** (ENH-038) so renderers blit on scroll and redraw only changed rows. Python per-cell colors follow the palette and OSC 10/11 defaults too (ARC-101), and the deprecated bridges are gone (`poll_events_legacy`, `Grid::erase_rectangle`). par-mux gains **held-dead panes that survive a daemon restart** (ARC-114), **`list-commands` discovery with held-state replay on registration** (ENH-037), **`pane-exited-replay`**, **`clear-history`**, and opt-in **per-pane hook-only endpoints** (ENH-039). Feature flags split (**`mux-bin`**, **`macro-yaml`**), `Cargo.lock` is committed and release builds are `--locked` (ARC-107), core diagnostics reach the `log` facade (ARC-111), the streaming server is load-hardened (QA-184/191/192/198), and a large audit batch lands security and robustness fixes (SEC-125–SEC-134). See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

Version 0.57.0 is a **minor release** focused on par-mux layout and lifecycle: panes can be **zoomed** (`resize-pane -Z`), reorganized with **`break-pane`/`join-pane`/`move-window`/`swap-window`**, and a pane whose child exits is now **held with its exit code** (remain-on-exit) and brought back with **`respawn-pane`**; `split-window -b` places a new pane before its target, and `pane-info` replies gain an optional trailing `cmd=<base64>` foreground-command token. The shared key encoder covers **modifyOtherKeys and the macOS option-key modes** (ENH-028; new `Terminal.encode_key` Python binding and FFI `terminal_encode_key_ex`), the stub generator emits **real constructor signatures** (ENH-030), FFI dirty-range readback is allocation-free (ENH-026), and par-mux telemetry is validated by a typed `TelemetryV1` schema that rejects future-dated samples (ENH-029, QA-156). See [CHANGELOG.md](CHANGELOG.md) for complete release notes.

Every other release is documented in [CHANGELOG.md](CHANGELOG.md).

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
- **C-Compatible FFI** - embedding API in `include/terminal_core.h` (`ptec_terminal_create`/`feed`/`dirty_ranges`/`read_row` render loop, key encoding, snapshots, observers; `SharedState`/`SharedCell`) for C/C++ and Swift apps — see the [FFI Guide](docs/FFI_GUIDE.md) and `make xcframework` for iOS

### Graphics Support

- **Sixel Graphics** - DEC VT340 compatible bitmap graphics with half-block rendering
- **iTerm2 Inline Images** - OSC 1337 protocol for PNG, JPEG, GIF images
- **Kitty Graphics Protocol** - APC G protocol with image reuse, animations, zlib compression (`o=z`), and advanced placement; the `t=f`/`t=t` file media is security-gated (`set_allow_file_media`, default allows only spec-named `*tty-graphics-protocol*` temp files, deleted only after decoding)
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

- **Multiple Formats** - PNG, JPEG, BMP, SVG (vector); HTML via `export_html()`
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

- **[Quick Start Guide](QUICKSTART.md)** - Get running in minutes
- **[API Reference](docs/API_REFERENCE.md)** - Complete Python API documentation
- **[VT Sequences](docs/VT_SEQUENCES.md)** - Comprehensive ANSI/VT sequence reference
- **[Advanced Features](docs/ADVANCED_FEATURES.md)** - Detailed feature guides
- **[Architecture](docs/ARCHITECTURE.md)** - Internal architecture details
- **[Security](docs/SECURITY.md)** - PTY security best practices
- **[Building](docs/BUILDING.md)** - Build instructions and requirements
- **[Troubleshooting](docs/TROUBLESHOOTING.md)** - Common build, test, streaming, and par-mux failures in one place
- **[Configuration Reference](docs/CONFIG_REFERENCE.md)** - Configuration options and the [environment variable reference](docs/CONFIG_REFERENCE.md#environment-variables)
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
- **[Benchmarking](docs/BENCHMARKING.md)** - Criterion throughput benches and interleaved A/B comparison
- **[Testing Kitty Animations](docs/TESTING_KITTY_ANIMATIONS.md)** - Testing Kitty graphics animation support
- **[Regional Flag Limitation](docs/REGIONAL_FLAG_LIMITATION.md)** - Why regional-indicator flag emoji may not render as one glyph in some frontends
- **[Maturin Best Practices](docs/MATURIN_BEST_PRACTICES.md)** - Wheel build and packaging compliance review
- **[par-mux Design Pointer](docs/par-mux.md)** - Where the par-mux design plan lives
- **[par-mux Design Decisions](docs/MUX_DECISIONS.md)** - One-line summaries of the D-numbered decisions cited in code
- **[Documentation Style Guide](docs/DOCUMENTATION_STYLE_GUIDE.md)** - Standards for writing project documentation

## Installation

### From PyPI

```bash
uv add par-term-emu-core-rust
# or
pip install par-term-emu-core-rust
```

Wheels ship with the streaming server compiled in (`StreamingServer` works out of the box; check `par_term_emu_core_rust._native.HAS_STREAMING` to detect a build without it).

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
# streaming is included: pyproject.toml [tool.maturin] features apply to every maturin build
maturin build --release
uv add --find-links target/wheels par-term-emu-core-rust
# or
pip install target/wheels/par_term_emu_core_rust-*.whl
```

### Using as a Rust Library

The library can be used in pure Rust projects without Python. Choose your feature combination:

| Use Case | Command | What's Included |
|----------|------------|-----------------|
| **Rust Only** | `cargo add par-term-emu-core-rust --no-default-features --features pty_session` | Terminal, PTY, Macros |
| **Rust + Streaming** | `cargo add par-term-emu-core-rust --no-default-features --features streaming,pty_session` | + WebSocket/HTTP server |
| **Python Only** | `cargo add par-term-emu-core-rust` | + Python bindings |
| **Everything** | `cargo add par-term-emu-core-rust --features full` | All features |

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

**Pre-built `par-mux` daemon binaries** (standalone: no Python, no par-term
installation) are attached to the same [GitHub Releases](https://github.com/paulrobello/par-term-emu-core-rust/releases)
as `par-mux-v<version>-<target>.tar.gz` (Unix) / `.zip` (Windows) archives
with a `SHA256SUMS` file. Per-platform install steps:
[docs/MUX.md](docs/MUX.md#standalone-binaries-github-releases).

**Or install from crates.io:**
```bash
# Streaming server (no-default-features is required: the default python feature
# links libpython and fails at link time for a standalone binary)
cargo install par-term-emu-core-rust --no-default-features --features streaming-bin --bin par-term-streamer

# Terminal multiplexer daemon, latest published release (0.58.0, no attach client):
cargo install par-term-emu-core-rust --version 0.58.0 --no-default-features --features mux-bin --bin par-mux
```

The `par-mux attach` client and the standalone `par-mux` crate ship in 0.58.1
(pending). Until then, build them from a source checkout (drop `,attach` for
the daemon alone):

```bash
cargo install --path crates/par-mux --features mux-bin,attach --locked
```

**Or build from source:**
```bash
cargo build --bin par-term-streamer --no-default-features --features streaming-bin --release
./target/release/par-term-streamer --help
```

See [docs/RUST_USAGE.md](docs/RUST_USAGE.md) for detailed Rust API documentation and examples.

### Embedding in iOS Apps

The core ships a C API (`src/ffi.rs`, header `include/terminal_core.h`) for on-device embedding — the same emulator as par-term, packaged as a static xcframework for iOS device and simulator:

```bash
# Requires Xcode + rustup target add aarch64-apple-ios aarch64-apple-ios-sim
make xcframework
# → target/xcframework/TerminalCore.xcframework
```

`TerminalCore.xcframework` (device + simulator slices) is attached to GitHub releases from 0.56.0 on; build it locally with `make xcframework` for unreleased commits. Each slice carries a `Modules/module.modulemap`, so after adding the xcframework to an Xcode project a Swift file just does `import TerminalCore` (the C header is re-exported; no bridging header needed).

The C surface is a full embedding API, not just snapshots (contracts and examples in the [FFI Guide](docs/FFI_GUIDE.md)):

- **Version check**: `ptec_terminal_abi_version` — compare against the header's `TERM_CORE_ABI_VERSION` at startup
- **Lifecycle/input**: `ptec_terminal_create` / `ptec_terminal_free` / `ptec_terminal_feed` (VT bytes) / `ptec_terminal_resize`
- **Damage**: `ptec_terminal_dirty_ranges` returns coalesced inclusive dirty-row ranges and `ptec_terminal_mark_clean` consumes them; independent renderers use `ptec_terminal_damage_generation` + `ptec_terminal_dirty_ranges_since`, so one consumer never hides damage from another
- **Scroll-aware damage (ABI v5)**: `ptec_terminal_scroll_delta_since` + `ptec_terminal_content_dirty_ranges_since` report a region's net scroll plus only the content-changed rows, so a renderer blits on scroll and redraws just the new rows
- **Pinned readback**: `ptec_terminal_read_row` / `ptec_terminal_read_scrollback_row` / `ptec_terminal_scrollback_count` copy cells into caller-owned buffers — no allocation, no full-grid copy per frame — with palette-resolved colors and default-color bits (ABI v4); `ptec_terminal_read_cell_grapheme` returns a cell's full grapheme cluster; `ptec_terminal_scrollback_total_scrolled` pairs with the count to sync a mirrored scrollback window; `ptec_terminal_get_cursor` / `ptec_terminal_get_modes` carry per-frame state
- **Key encoding**: `ptec_terminal_encode_key` turns key events into PTY bytes (xterm legacy, kitty level-1 disambiguate, and modifyOtherKeys, honoring application cursor keys and the negotiated kitty flags); `ptec_terminal_encode_key_ex` adds per-side macOS Option-key modes through `TermKeyOptions`
- **Snapshots and observers**: `ptec_terminal_get_state` / `ptec_terminal_free_state`, `ptec_terminal_add_observer` / `ptec_terminal_remove_observer`; the `on_event_v2` slot delivers structured `TermEvent`s (a `TERM_EVENT_*` kind plus a JSON payload)

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

# Create PTY terminal and spawn shell
with PtyTerminal(80, 24) as term:
    term.spawn_shell()

    # Send commands
    term.write_str("echo 'Hello from shell!'\n")
    # Block until the output actually lands — no sleep guessing
    term.wait_for_text("Hello from shell!", timeout=3.0)

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
open("output.html", "w").write(term.export_html(include_styles=True))  # Styled HTML

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

### Diagnostics and Test Scripts
- `bce_scroll_test.py` - Background color erase (BCE) during scrolling
- `char_test.py` - Character-specific rendering test
- `gradient_test.py` - Minimal gradient test for terminal wrap behavior
- `rich_mimic_test.py` - Rich-style output rendering test
- `scroll_timing_test.py` - Scroll timing test
- `streaming_debug.py` - Streaming server debug tool
- `test_tui_clipboard.py` - TUI clipboard and selection features
- `test_underline_styles.py` - Underline styles (SGR 4:x)
- `render_utils.py` - Rendering helpers imported by the other examples (not run directly)

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
# Install dependencies (bun)
make web-install

# Development server on http://localhost:3000
make web-dev

# Static build copied to web_term/ for par-term-streamer --web-root
make web-build-static
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
# All tests: Rust, Rust streaming, and Python (rebuilds the extension first)
make test

# Rust tests only (plain `cargo test` fails to link under the default `python` feature)
make test-rust

# Python tests only (runs `make dev` first)
make test-python
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

See [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution workflow, and [CLAUDE.md](CLAUDE.md) for detailed development instructions.

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
