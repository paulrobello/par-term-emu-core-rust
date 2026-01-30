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

## What's New in 0.22.1

### 🐛 Search Unicode Bug Fix

Fixed `search()` and `search_scrollback()` returning byte offsets instead of character offsets for text containing multi-byte Unicode characters (CJK, emoji, etc.):

- `SearchMatch.col` now correctly returns the character column position
- `SearchMatch.length` now correctly returns the character count
- Example: Searching for "World" in "こんにちは World" now returns `col=6` (correct) instead of `col=16` (byte offset)

## What's New in 0.22.0

### 🏳️ Regional Indicator Flag Emoji Support

Proper grapheme cluster handling for flag emoji like 🇺🇸, 🇬🇧, 🇯🇵:

- Flag emoji are now correctly combined into single wide (2-cell) graphemes
- Two regional indicator codepoints are combined with the first as the base character and the second in the combining vector
- Cursor correctly advances by 2 cells after writing a flag
- Added `unicode-segmentation` crate dependency for grapheme cluster support
- Comprehensive test suite for flag emoji

## What's New in 0.21.0

### 🚀 parking_lot Migration

The entire library has been migrated from `std::sync::Mutex` to **`parking_lot::Mutex`**.

- **Improved Reliability**: Eliminated "Mutex Poisoning". A panic in one thread no longer renders the terminal state permanently inaccessible to other threads.
- **Better Performance**: Faster lock/unlock operations and significantly smaller memory footprint for locks.
- **Ergonomic API**: Lock acquisition no longer requires `.unwrap()`, making the code cleaner and more robust.

## What's New in 0.20.1

### 🔧 Safe Environment Variable API

Added new methods to pass environment variables and working directory directly to spawned processes without modifying the global environment of the parent process.

- **Rust**: `spawn_shell_with_env(env, cwd)`, `spawn_with_env(command, args, env, cwd)`
- **Python**: `spawn_shell(env=None, cwd=None)` - now supports optional environment dictionary and working directory path.
- **Thread Safety**: Eliminates the need for `unsafe { std::env::set_var() }` in multi-threaded applications like those using Tokio.

## What's New in 0.20.0

### 🎨 External UI Theme

The web frontend UI chrome can now be customized **after static build** without rebuilding:

```css
/* Edit web_term/theme.css */
:root {
  --terminal-bg: #0a0a0a;      /* Main background */
  --terminal-surface: #1a1a1a; /* Status bar, cards */
  --terminal-border: #2a2a2a;  /* Borders */
  --terminal-accent: #3a3a3a;  /* Scrollbar, accents */
  --terminal-text: #e0e0e0;    /* Primary text */
}
```

- Edit colors and refresh the page - no rebuild required
- Terminal emulator colors (ANSI palette) still controlled by server `--theme` option
- See [docs/STREAMING.md](docs/STREAMING.md#theme-system) for details

### 🐛 Bug Fixes

- **Web Terminal On-Screen Keyboard**: Fixed native device keyboard appearing when tapping on-screen keyboard buttons on mobile devices
  - The on-screen keyboard now properly prevents xterm's internal textarea from gaining focus
  - Tapping virtual keys no longer triggers the device's native keyboard

## What's New in 0.19.5

### 🐛 Bug Fixes

- **Streaming Server Shell Restart Input**: Fixed WebSocket client connections not receiving input after shell restart
  - PTY writer was captured once at connection time, becoming stale after shell restart
  - Client keyboard input now properly reaches the shell after any restart

## What's New in 0.19.4

### 🔧 Python SDK Sync

- **Python SDK aligned with Rust SDK**: All streaming features now available in Python bindings
  - `StreamingConfig.enable_http` / `web_root` - HTTP server configuration (getter/setter)
  - `StreamingServer.max_clients()` - Query maximum allowed clients
  - `StreamingServer.create_theme_info()` - Create theme dictionaries for protocol
  - `encode_server_message("pong")` - Pong message encoding support
  - `encode_server_message("connected", theme=...)` - Theme support in connected messages

```python
from par_term_emu_core_rust import StreamingConfig, StreamingServer, encode_server_message

# Configure HTTP serving
config = StreamingConfig(enable_http=True, web_root="/var/www/terminal")

# Create theme for connected message
theme = StreamingServer.create_theme_info(
    name="my-theme",
    background=(0, 0, 0),
    foreground=(255, 255, 255),
    normal=[(0,0,0), (255,0,0), (0,255,0), (255,255,0), (0,0,255), (255,0,255), (0,255,255), (200,200,200)],
    bright=[(128,128,128), (255,128,128), (128,255,128), (255,255,128), (128,128,255), (255,128,255), (128,255,255), (255,255,255)]
)

# Encode messages
pong = encode_server_message("pong")
connected = encode_server_message("connected", cols=80, rows=24, session_id="abc", theme=theme)
```

## What's New in 0.19.2

### 🐛 Bug Fixes

- **Streaming Server Hang on Shell Exit**: Fixed server hanging indefinitely when the shell exits
  - Added shutdown signal mechanism to gracefully terminate the broadcaster loop
  - Prevents blocking indefinitely when shell exits in some conditions

## What's New in 0.19.1

### 🐛 Bug Fixes

- **Streaming Server Ping/Pong**: Fixed application-level ping/pong handling
  - Server was sending WebSocket-level pong frames instead of protobuf `Pong` messages
  - Frontend heartbeat mechanism now properly receives pong responses
  - Fixes stale connection detection that was failing due to missing pong responses

## What's New in 0.19.0

### 🎉 New Features

- **Automatic Shell Restart**: Streaming server now automatically restarts the shell when it exits
  - Default behavior: shell is restarted automatically when it exits
  - New `--no-restart-shell` CLI option to disable automatic restart
  - New `PAR_TERM_NO_RESTART_SHELL` environment variable support
  - When restart is disabled, server exits gracefully when the shell exits

- **Header/Footer Toggle in On-Screen Keyboard**: Layout toggle button in keyboard header
  - Show/hide header and footer directly from the on-screen keyboard
  - Blue indicator shows when header/footer is visible
  - Convenient for maximizing terminal space on mobile

- **Font Size Controls in On-Screen Keyboard**: Plus/minus buttons in keyboard header
  - Adjust font size (8-32px) without opening the header panel

### 🔧 Changes

- **StreamingServer API**: `set_pty_writer` now uses interior mutability for shell restart support
- **UI Improvements**: Font size controls moved to keyboard header; floating buttons repositioned side by side

## What's New in 0.18.2

### 🎉 New Features

- **Font Size Control**: User-adjustable terminal font size in web frontend
  - Plus/minus buttons in header (8px to 32px range)
  - Persisted to localStorage across sessions

- **Heartbeat/Ping Mechanism**: Stale WebSocket connection detection
  - Sends ping every 25s, expects pong within 10s
  - Automatically closes and reconnects stale connections

### 🔒 Security Hardening

- **Web Terminal Security Fixes**: Comprehensive security audit remediation
  - **Reverse-tabnabbing prevention**: Terminal links now open with `noopener,noreferrer`
  - **Zip bomb protection**: Added decompression size limits (256KB compressed, 2MB decompressed)
  - **Localhost probe fix**: WebSocket preconnect hints gated to development mode only
  - **Snapshot size guard**: 1MB limit on screen snapshots to prevent UI freezes

### 🐛 Bug Fixes

- **WebSocket URL Changes**: Properly disconnects and reconnects when URL changes
- **Invalid URL Handling**: Displays friendly error instead of crashing
- **Next.js Config**: Merged duplicate config files into single file
- **Toggle Button Overlap**: Moved button left to avoid scrollbar overlap

## What's New in 0.18.1

### 🐛 Bug Fixes

- **Web Terminal On-Screen Keyboard**: Fixed device virtual keyboard appearing when tapping on-screen keyboard buttons on mobile devices
  - Added `tabIndex={-1}` to all buttons to prevent focus acquisition that triggered device keyboard

## What's New in 0.18.0

### 🎉 New Features

- **Environment Variable Support**: All CLI options now support environment variables with `PAR_TERM_` prefix
  - Examples: `PAR_TERM_HOST`, `PAR_TERM_PORT`, `PAR_TERM_THEME`, `PAR_TERM_HTTP_USER`
  - Configuration via environment for containerized deployments

- **HTTP Basic Authentication**: New password protection for the web frontend
  - `--http-user` - Username for HTTP Basic Auth
  - `--http-password` - Clear text password
  - `--http-password-hash` - htpasswd format hash (bcrypt, apr1, SHA1, MD5 crypt)
  - `--http-password-file` - Read password from file (auto-detects hash vs clear text)

### 🧪 Test Coverage

- **Comprehensive Streaming Test Suite**: 94 new tests for streaming functionality
  - Protocol message constructors, theme info, HTTP Basic Auth configuration
  - Binary protocol encoding/decoding with compression
  - Event types, streaming errors, JSON serialization
  - Unicode content and ANSI escape sequence preservation

### 🔧 Improvements

- **Python Bindings**: Binary protocol functions now properly exported (`encode_server_message`, `decode_server_message`, `encode_client_message`, `decode_client_message`)

### Usage Examples

```bash
# Environment variables
export PAR_TERM_HOST=0.0.0.0
export PAR_TERM_HTTP_USER=admin
export PAR_TERM_HTTP_PASSWORD=secret
par-term-streamer --enable-http

# CLI with htpasswd hash
par-term-streamer --enable-http --http-user admin --http-password-hash '$apr1$...'
```

## What's New in 0.17.0

### 🎉 New Features

- **Web Terminal Macro System**: New macro tab in the on-screen keyboard for creating and playing terminal command macros
  - Create named macros with multi-line scripts (one command per line)
  - Quick select buttons to run macros with a single tap
  - Playback with 200ms delay before each Enter key for reliable command execution
  - Edit and delete existing macros via hover menu
  - Stop button to abort macro playback mid-execution
  - Macros persist to localStorage across sessions
  - Visual feedback during playback (pulsing animation, stop button)
  - Option to disable sending Enter after each line (for text insertion macros)
  - Template commands for advanced scripting: `[[delay:N]]`, `[[enter]]`, `[[tab]]`, `[[esc]]`, `[[space]]`, `[[ctrl+X]]`, `[[shift+X]]`, `[[ctrl+shift+X]]`, `[[shift+tab]]`, `[[shift+enter]]`

- **On-Screen Keyboard Enhancements**:
  - Permanent symbols grid on the right side with all keyboard symbols (32 keys)
  - Added Space, Enter, http://, and https:// buttons to modifier row
  - Added tooltips to Ctrl shortcut buttons
  - Expanded symbol keys with full punctuation set

### 🔧 Improvements

- **On-Screen Keyboard Layout**: Reorganized for better usability with more compact vertical layout and persistent symbols grid

### 📦 Dependency Updates

- **Web Frontend**: Updated @types/node (25.0.1 → 25.0.2)

## What's New in 0.16.3

### 🐛 Bug Fixes

- **Web Terminal tmux/TUI Fix**: Fixed control characters (`^[[?1;2c^[[>0;276;0c`) appearing when running tmux or other TUI applications in the web terminal. The issue was caused by xterm.js generating Device Attributes responses when the backend terminal emulator already handles these queries.

### 🚀 Performance Optimizations

- **jemalloc Allocator**: New optional `jemalloc` feature for 5-15% server throughput improvement (non-Windows only)
- **TCP_NODELAY**: Disabled Nagle's algorithm for lower keystroke latency (up to 40ms improvement)
- **Output Batching**: Time-based batching at 60fps reduces WebSocket overhead by 50-80% during burst output
- **Compression Threshold**: Lowered to 256 bytes to compress more typical terminal output
- **WebSocket Preconnect**: Reduces initial connection latency by 100-200ms
- **Font Preloading**: Eliminates layout shift and font flash

### 📦 Dependency Updates

- **Web Frontend**: Updated Next.js and type definitions
- **Pre-commit Hooks**: Updated ruff linter

## What's New in 0.16.2

### 🔧 Compatibility Fix

- **TERM Environment Variable**: Changed default `TERM` from `xterm-kitty` to `xterm-256color` for better compatibility with systems lacking kitty terminfo

## What's New in 0.16.0

### 🔒 TLS/SSL Support

- **Secure WebSocket Connections** for production deployments:
  - New CLI options: `--tls-cert`, `--tls-key`, `--tls-pem`
  - Supports separate cert/key files or combined PEM
  - Enables HTTPS and WSS (secure WebSocket)

```bash
# Using separate cert and key files
par-term-streamer --enable-http --tls-cert cert.pem --tls-key key.pem

# Using combined PEM file
par-term-streamer --enable-http --tls-pem combined.pem
```

### 🚀 Performance: Binary Protocol

- **BREAKING: Protocol Buffers for WebSocket Streaming**:
  - Replaced JSON with binary Protocol Buffers encoding
  - **~80% smaller messages** for typical terminal output
  - Optional zlib compression for large payloads (screen snapshots)
  - Wire format: 1-byte header + protobuf payload

### 🐍 Python Bindings

- **TLS Configuration**: `StreamingConfig` methods for TLS setup
- **Binary Protocol Functions**: `encode_server_message()`, `decode_server_message()`, `encode_client_message()`, `decode_client_message()`

See [CHANGELOG.md](CHANGELOG.md) for complete version history.

## What's New in 0.15.0

### 🎉 New Features

- **Streaming Server CLI Enhancements**:
  - `--download-frontend` option to download prebuilt web frontend from GitHub releases
  - `--frontend-version` option to specify version to download (default: "latest")
  - `--use-tty-size` option to use current terminal size from TTY
  - No longer requires Node.js/npm to use web frontend - can download prebuilt version

### Quick Start

```bash
# Build the streaming server
make streamer-build-release

# Download prebuilt web frontend (no Node.js required!)
./target/release/par-term-streamer --download-frontend

# Run server with frontend
./target/release/par-term-streamer --enable-http

# Open browser to http://127.0.0.1:8099
```

## What's New in 0.14.0

### 🎉 New Features

- **Web Terminal Onscreen Keyboard**: Mobile-friendly virtual keyboard for touch devices
  - Special keys missing from iOS/Android keyboards: Esc, Tab, arrow keys, Page Up/Down, Home, End, Insert, Delete
  - Function keys F1-F12 (toggleable), symbol keys (|, \, `, ~, {, }, etc.)
  - Modifier keys (Ctrl, Alt, Shift) that combine with other keys
  - Quick Ctrl shortcuts: ^C, ^D, ^Z, ^L, ^A, ^E, ^K, ^U, ^W, ^R
  - Glass morphism design, haptic feedback, auto-shows on mobile

- **OSC 9;4 Progress Bar Support** (ConEmu/Windows Terminal style):
  - Terminal applications can report progress that can be displayed in tab bars, taskbars, or window titles

## What's New in 0.13.0

### 🎉 New Features
- **Streaming Server Enhancements**:
  - `--size` CLI option for specifying terminal size in `COLSxROWS` format (e.g., `--size 120x40` or `-s 120x40`)
  - `--command` / `-c` CLI option to execute a command after shell startup (with 1 second delay for prompt settling)
  - `initial_cols` and `initial_rows` configuration options in `StreamingConfig` for both Rust and Python APIs

- **Python Bindings Enhancements**:
  - New `MouseEncoding` enum for mouse event encoding control (Default, Utf8, Sgr, Urxvt)
  - Direct screen buffer control: `use_alt_screen()`, `use_primary_screen()`
  - Mouse encoding control: `mouse_encoding()`, `set_mouse_encoding()`
  - Mode setters: `set_focus_tracking()`, `set_bracketed_paste()`, `set_title()`
  - Bold brightening control: `bold_brightening()`, `set_bold_brightening()`
  - Color getters for all theme colors (link, bold, cursor guide, badge, match, selection)

## What's New in 0.12.0

### 🐛 Bug Fixes
- **Terminal Reflow Improvements**: Multiple fixes to scrollback and grid reflow behavior during resize

## What's New in 0.11.0

### 🎉 New Features
- **Full Terminal Reflow on Width Resize**: Both scrollback AND visible screen content now reflow when terminal width changes
  - Previously, width changes cleared scrollback and clipped visible content
  - Now implements intelligent reflow similar to xterm and iTerm2:
    - **Scrollback**: Preserves all history with proper line wrapping/unwrapping
    - **Visible Screen**: Content wraps instead of being clipped when narrowing
    - Width increase: Unwraps soft-wrapped lines into longer lines
    - Width decrease: Re-wraps lines that no longer fit
  - Preserves all cell attributes (colors, bold, italic, etc.)
  - Handles wide characters (CJK, emoji) correctly at line boundaries
  - Significant UX improvement for terminal resize operations

## What's New in 0.10.0

### 🎉 New Features
- **Emoji Sequence Preservation**: Complete support for complex emoji sequences and grapheme clusters
  - ⚠️ vs ⚠ - Variation selectors (emoji vs text style)
  - 👋🏽 - Skin tone modifiers (Fitzpatrick scale)
  - 👨‍👩‍👧‍👦 - ZWJ sequences (family emoji)
  - 🇺🇸 🇬🇧 - Regional indicator flags
  - é - Combining diacritics and marks
  - New `grapheme` module for Unicode cluster detection
  - Enhanced Python bindings export full grapheme clusters

- **Web Terminal Frontend**: Modern Next.js-based web interface
  - Built with Next.js, TypeScript, and Tailwind CSS v4
  - Theme support with configurable color palettes
  - Nerd Font support for file/folder icons
  - New Makefile targets for web frontend development

- **Terminal Sequence Support**:
  - CSI 3J - Clear scrollback buffer command
  - Improved cursor positioning for snapshot exports

### 🐛 Bug Fixes
- Graphics now properly preserved when scrolling into scrollback buffer
- Sixel content saved to scrollback during large scrolling operations
- Kitty Graphics Protocol animation parsing fixes (base64 encoding, frame actions)

### ⚠️ Breaking Changes (Rust API only)
- **`Cell` struct no longer implements `Copy`** (now `Clone` only)
  - Required for variable-length grapheme cluster storage
  - All cell copy operations now require explicit `.clone()` calls
  - **Python bindings are unaffected** - no changes needed in Python code
  - Performance impact is minimal due to efficient cloning

## What's New in 0.9.1

- **Theme Rendering Fix**: Fixed theme color palette application in Python bindings

## What's New in 0.9.0

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

See [CHANGELOG.md](CHANGELOG.md) for complete version history.

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
- **OSC 9/777 Notifications** - Desktop-style alerts and notifications
- **Shell Integration** - OSC 133 (iTerm2/VSCode compatible)
- **Kitty Keyboard Protocol** - Progressive keyboard enhancement with auto-reset on alternate screen exit
- **Synchronized Updates (DEC 2026)** - Flicker-free rendering
- **Tmux Control Protocol** - Control mode integration support

### Graphics Support

- **Sixel Graphics** - DEC VT340 compatible bitmap graphics with half-block rendering
- **iTerm2 Inline Images** - OSC 1337 protocol for PNG, JPEG, GIF images
- **Kitty Graphics Protocol** - APC G protocol with image reuse, animations, and advanced placement
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
- **[Rust Usage](docs/RUST_USAGE.md)** - Using the library in pure Rust projects
- **[Graphics Testing](docs/GRAPHICS_TESTING.md)** - Testing graphics protocol implementations

## Installation

### From PyPI

```bash
uv add par-term-emu-core-rust
# or
pip install par-term-emu-core-rust
```

### From Source

Requires Rust 1.75+ and Python 3.12+:

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
| **Rust Only** | `par-term-emu-core-rust = { version = "0.10", default-features = false }` | Terminal, PTY, Macros |
| **Rust + Streaming** | `par-term-emu-core-rust = { version = "0.10", default-features = false, features = ["streaming"] }` | + WebSocket/HTTP server |
| **Python Only** | `par-term-emu-core-rust = "0.10"` | + Python bindings |
| **Everything** | `par-term-emu-core-rust = { version = "0.10", features = ["full"] }` | All features |

**Download pre-built streaming server (recommended):**

Pre-built binaries and web frontend packages are available from [GitHub Releases](https://github.com/paulrobello/par-term-emu-core-rust/releases):

```bash
# Download binary (Linux example)
wget https://github.com/paulrobello/par-term-emu-core-rust/releases/latest/download/par-term-streamer-linux-x86_64
chmod +x par-term-streamer-linux-x86_64

# Download web frontend
wget https://github.com/paulrobello/par-term-emu-core-rust/releases/latest/download/par-term-web-frontend-v0.10.0.tar.gz
tar -xzf par-term-web-frontend-v0.10.0.tar.gz -C ./web_term

# Run
./par-term-streamer-linux-x86_64 --web-root ./web_term
```

Available binaries: Linux (x86_64, ARM64), macOS (Intel, Apple Silicon), Windows (x86_64)

**Or install from crates.io:**
```bash
cargo install par-term-emu-core-rust --features streaming
```

**Or build from source:**
```bash
cargo build --bin par-term-streamer --no-default-features --features streaming --release
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
    minimum_contrast=0.5  # iTerm2-compatible contrast adjustment
)
```

### Color Utilities

```python
from par_term_emu_core_rust import (
    perceived_brightness_rgb, adjust_contrast_rgb,
    contrast_ratio, meets_wcag_aa,
    rgb_to_hex, hex_to_rgb, mix_colors
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
# Build and run (default: ws://127.0.0.1:8080)
make streamer-run

# Run with authentication
make streamer-run-auth

# Or use cargo directly
cargo build --bin par-term-streamer --no-default-features --features streaming --release
./target/release/par-term-streamer --port 8080 --theme dracula

# With authentication
./target/release/par-term-streamer --api-key my-secret --theme monokai

# Install globally
make streamer-install
par-term-streamer --help
```

**Available Themes:** `iterm2-dark`, `monokai`, `dracula`, `solarized-dark`

### Web Terminal Frontend

**Using Pre-built Package (Recommended):**

Download the pre-built static web frontend from [GitHub Releases](https://github.com/paulrobello/par-term-emu-core-rust/releases):

```bash
# Download and extract
wget https://github.com/paulrobello/par-term-emu-core-rust/releases/latest/download/par-term-web-frontend-v0.10.0.tar.gz
tar -xzf par-term-web-frontend-v0.10.0.tar.gz -C ./web_term

# Run streamer with web frontend
par-term-streamer --web-root ./web_term
# Open browser to http://localhost:8080
```

See [web_term/README.md](web_term/README.md) for detailed usage instructions.

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

- **Rust** (1.75+) - Core library implementation
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
