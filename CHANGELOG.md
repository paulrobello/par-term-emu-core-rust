# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
