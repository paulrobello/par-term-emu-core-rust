# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
