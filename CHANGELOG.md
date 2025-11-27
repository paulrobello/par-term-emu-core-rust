# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.11.0] - 2025-11-26

### Added
- **Scrollback Reflow on Width Resize**: Scrollback content is now preserved and properly reflowed when terminal width changes
  - Previously, changing terminal width would clear all scrollback to avoid panics from misaligned cell indexing
  - Now implements intelligent reflow similar to xterm and iTerm2:
    - **Width increase**: Unwraps previously soft-wrapped lines into longer lines
    - **Width decrease**: Re-wraps lines that no longer fit
  - Preserves all cell attributes (colors, bold, italic, etc.) during reflow
  - Handles wide characters (CJK, emoji) correctly at line boundaries
  - Properly manages circular buffer during reflow
  - Respects max_scrollback limits when reflow creates additional lines
  - Significant UX improvement for terminal resize operations

### Changed
- Height-only resize operations no longer trigger scrollback reflow (optimization)
- Scrollback buffer is now rebuilt (non-circular) after reflow for simpler indexing

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
