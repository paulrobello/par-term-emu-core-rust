# Architecture Documentation

This document describes the internal architecture of par-term-emu-core-rust.

## Table of Contents

- [Overview](#overview)
- [Core Components](#core-components)
  - [1. Color](#1-color)
  - [2. Cell](#2-cell)
  - [3. Cursor](#3-cursor)
  - [4. Grid](#4-grid)
  - [5. Terminal](#5-terminal)
- [ANSI Sequence Processing](#ansi-sequence-processing)
- [Data Flow](#data-flow)
- [Python Bindings](#python-bindings)
- [Memory Management](#memory-management)
- [Performance Considerations](#performance-considerations)
- [Extension Points](#extension-points)
- [Testing Strategy](#testing-strategy)
- [Dependencies](#dependencies)
- [Build Process](#build-process)
- [Debugging](#debugging)
- [Contributing](#contributing)
- [References](#references)

## Overview

par-term-emu-core-rust is a terminal emulator library written in Rust with Python bindings. It uses the VTE (Virtual Terminal Emulator) crate for ANSI sequence parsing and PyO3 for Python interoperability.

## Core Components

### 1. Color

**Location:** `src/color.rs`

Represents colors in various formats:

- **Named Colors**: 16 basic ANSI colors (black, red, green, etc.)
- **Indexed Colors**: 256-color palette (0-255)
- **RGB Colors**: 24-bit true color (r, g, b)

All colors can be converted to RGB for rendering.

```rust
pub enum Color {
    Named(NamedColor),
    Indexed(u8),
    Rgb(u8, u8, u8),
}
```

### 2. Cell

**Location:** `src/cell.rs`

Represents a single character cell in the terminal grid. Each cell contains:

- A character (Unicode)
- Foreground color
- Background color
- Text attributes (bold, italic, underline, etc.)

```rust
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Option<Color>,  // SGR 58/59
    pub flags: CellFlags,
}
```

### 3. Cursor

**Location:** `src/cursor.rs`

Tracks the cursor state:

- Position (col, row)
- Visibility (shown/hidden)

Provides methods for cursor movement and positioning.

### 4. Grid

**Location:** `src/grid.rs`

Manages the 2D terminal buffer:

- Main screen buffer (cols × rows)
- Scrollback buffer (configurable size)
- Scrolling operations
- Cell access and manipulation
- Resize handling

The grid uses a flat Vec for efficient storage and access:

```rust
pub struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,              // Row-major order
    scrollback_cells: Vec<Cell>,   // Flat circular buffer
    scrollback_start: usize,       // Circular buffer head
    scrollback_lines: usize,       // Current scrollback count
    max_scrollback: usize,
    wrapped: Vec<bool>,            // Line wrap tracking
    scrollback_wrapped: Vec<bool>, // Scrollback wrap tracking
}
```

### 5. Terminal

**Location:** `src/terminal/mod.rs` (modular implementation)

The main terminal emulator that ties everything together, organized into submodules:
- `notification.rs` - Notification types from OSC sequences
- `sequences/` - VTE sequence handlers (CSI, OSC, ESC, DCS)
- `graphics.rs` - Sixel graphics management
- `colors.rs` - Color configuration
- `write.rs` - Character writing logic

Features:

- Owns the grid and cursor
- Implements the VTE `Perform` trait for ANSI parsing
- Manages terminal state (colors, attributes)
- Handles all terminal operations

### 6. Supporting Modules

**Mouse Handling** (`src/mouse.rs`)
- Mouse event types and button tracking
- Mouse mode management (Normal, Button, Any)
- Mouse encoding formats (SGR, UTF-8, URXVT)

**Shell Integration** (`src/shell_integration.rs`)
- OSC 133 prompt/command/output markers
- Command execution tracking
- Integration with modern shells (fish, zsh, bash)

**Sixel Graphics** (`src/sixel.rs`)
- Sixel image parser and decoder
- Graphics storage and positioning
- Half-block and pixel rendering modes

**Utility Modules**
- `ansi_utils.rs` - ANSI sequence parsing and generation helpers
- `color_utils.rs` - Color conversion and manipulation utilities
- `text_utils.rs` - Text processing and Unicode handling
  - Word boundary detection with configurable word characters
  - Default word characters: `"/-+\\~_."` (iTerm2-compatible)
  - `DEFAULT_WORD_CHARS` constant for word selection
  - `is_word_char()`, `get_word_at()`, `select_word()` functions
- `html_export.rs` - HTML export functionality for terminal content
  - Complete HTML document generation with embedded styles
  - Scrollback buffer export support
  - Inline CSS for terminal styling
  - Color preservation (foreground, background, attributes)
  - Monospace font stack: Monaco, Menlo, Ubuntu Mono, Consolas, monospace
- `debug.rs` - Debug utilities and logging helpers

**PTY Support**
- `pty_session.rs` - PTY session management with portable-pty
- `pty_error.rs` - PTY-specific error types

`PtySession` owns:

- An `Arc<Mutex<Terminal>>` for all terminal state.
- A `portable_pty::PtyPair` and child process handle.
- A background reader thread that:
  - Reads from the PTY master.
  - Feeds bytes into the `Terminal` via `term.process(..)` while holding the terminal mutex.
  - Writes device-query responses (DA/DSR/DECRQM/etc.) back to the child via the shared writer.
- An `Arc<AtomicBool>` `running` flag that reflects the session’s view of whether the child is still alive.

`running` is deliberately a **best-effort** indicator:

- It is set to `true` when a child is successfully spawned.
- It is set to `false` when:
  - EOF is observed on the PTY reader (reader thread).
  - `try_wait()` observes an exited child.
  - `wait()` completes.
  - `kill()` is called.
- There may be a short window where the OS still considers the process live even though `running == false`, or vice versa (between a process exit and the reader thread seeing EOF). Callers that need precise exit status should use `try_wait()`/`wait()` instead of relying solely on `is_running()`.

```rust
pub struct Terminal {
    // Screen management
    grid: Grid,
    alt_grid: Grid,
    alt_screen_active: bool,

    // Cursor state
    cursor: Cursor,
    alt_cursor: Cursor,
    saved_cursor: Option<Cursor>,

    // Text attributes
    fg: Color,
    bg: Color,
    underline_color: Option<Color>,
    flags: CellFlags,

    // Terminal modes
    mouse_mode: MouseMode,
    mouse_encoding: MouseEncoding,
    focus_tracking: bool,
    bracketed_paste: bool,
    synchronized_updates: bool,
    auto_wrap: bool,
    origin_mode: bool,
    application_cursor: bool,

    // Advanced features
    shell_integration: ShellIntegration,
    hyperlinks: HashMap<u32, String>,
    graphics: Vec<SixelGraphic>,
    keyboard_flags: u16,

    // Margins and regions
    scroll_region_top: usize,
    scroll_region_bottom: usize,
    left_margin: usize,
    right_margin: usize,
    use_lr_margins: bool,

    // ... additional state ...
}
```

## ANSI Sequence Processing

The terminal uses the VTE crate for parsing ANSI escape sequences:

```mermaid
graph LR
    A[Input bytes]
    B[VTE Parser]
    C[Perform callbacks]
    D[Terminal state updates]

    A --> B
    B --> C
    C --> D

    style A fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style B fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style C fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style D fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
```

The `Terminal` struct implements the `Perform` trait with these methods:

- `print(char)`: Handle printable characters
- `execute(byte)`: Handle C0 control codes (newline, tab, etc.)
- `csi_dispatch()`: Handle CSI sequences (cursor movement, colors, etc.)
- `osc_dispatch()`: Handle OSC sequences (terminal title, etc.)

## Data Flow

```mermaid
graph TD
    A[Python Code]
    B[PyO3 Bindings<br/>src/python_bindings/]
    C[Terminal::process<br/>src/terminal/mod.rs]
    D[VTE Parser]
    E[Perform Trait Methods<br/>src/terminal/sequences/]
    F[Grid/Cursor Updates<br/>src/grid.rs, src/cursor.rs]
    G[State Changes]
    H[Python API queries<br/>src/python_bindings/]

    A --> B
    B --> C
    C --> D
    D --> E
    E --> F
    F --> G
    G --> H
    H --> A

    style A fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style B fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style C fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style D fill:#1a237e,stroke:#3f51b5,stroke-width:2px,color:#ffffff
    style E fill:#37474f,stroke:#78909c,stroke-width:2px,color:#ffffff
    style F fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
    style G fill:#2e7d32,stroke:#66bb6a,stroke-width:2px,color:#ffffff
    style H fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
```

## Python Bindings

The Python bindings are organized in `src/python_bindings/` with multiple submodules:
- `terminal.rs` - PyTerminal struct and its implementation
- `pty.rs` - PyPtyTerminal struct and its implementation (PTY support)
- `types.rs` - Data types (PyAttributes, PyScreenSnapshot, PyShellIntegration, PyGraphic)
- `enums.rs` - Enum types (PyCursorStyle, PyUnderlineStyle)
- `conversions.rs` - Type conversions and parsing utilities

The main Python module is defined in `src/lib.rs`, which exports the `_native` module.

```rust
#[pyclass(name = "Terminal")]
pub struct PyTerminal {
    inner: crate::terminal::Terminal,
}
```

All public methods are wrapped with `#[pymethods]` and provide:

- Type conversion (Rust ↔ Python)
- Error handling (Result → PyResult)
- Pythonic API design

## Memory Management

- **Rust Side**: Owned data structures with automatic memory management
- **Python Side**: Python objects wrapping Rust data
- **Zero-copy**: Where possible, data is referenced rather than copied
- **Scrollback**: Limited by `max_scrollback` to prevent unbounded growth

## Performance Considerations

### Efficient Grid Storage

- Flat Vec for cache-friendly access
- Row-major order for sequential line access
- Minimal allocations during normal operation

### ANSI Parsing

- VTE crate provides fast, zero-allocation parsing
- State machine approach for streaming input

### Python Boundary

- Minimize Python/Rust crossings
- Batch operations where possible
- Return references instead of copying when safe

## Extension Points

### Adding New ANSI Sequences

1. Add handler in the appropriate sequence module:
   - CSI sequences: `src/terminal/sequences/csi.rs`
   - OSC sequences: `src/terminal/sequences/osc.rs`
   - ESC sequences: `src/terminal/sequences/esc.rs`
   - DCS sequences: `src/terminal/sequences/dcs.rs`
2. Update grid/cursor state as needed
3. Add tests

### New Color Formats

1. Add variant to `Color` enum in `src/color.rs`
2. Implement `to_rgb()` conversion
3. Update color handling in `src/terminal/sequences/csi.rs`

### Additional Cell Attributes

1. Add flag to `CellFlags` in `src/cell.rs`
2. Update SGR handling in `src/terminal/sequences/csi.rs`
3. Expose in Python API if needed (in `src/python_bindings/`)

## Testing Strategy

### Test Coverage

**Current test counts (as of latest commit):**
- **Rust tests:** 676 unit and integration tests
- **Python tests:** 267 tests across multiple test modules (34 skipped in CI)
- **Total:** 943 tests ensuring comprehensive coverage

### Rust Tests

- **Unit tests** in each module (included via `#[cfg(test)]` modules)
- **Integration tests** for full ANSI sequences and terminal operations
- **Property-based tests** for invariants (using `proptest` crate)
- **PyO3 configuration:** Tests run with `--no-default-features --features pyo3/auto-initialize`
  - The `extension-module` feature prevents linking during tests
  - Must use `auto-initialize` feature for test environment
  - Run via: `cargo test --lib --no-default-features --features pyo3/auto-initialize`

### Python Tests

- **API contract tests** validating Python bindings behavior
- **Example-based tests** covering common use cases
- **Edge case handling** for error conditions and boundary cases
- **Timeout protection:** 5-second default per test (10-second for slow tests)
- **PTY tests** excluded in CI (hang in automated environments):
  - `test_pty.py`, `test_ioctl_size.py`
  - `test_pty_resize_sigwinch.py`, `test_nested_shell_resize.py`

## Implemented Features

The terminal emulator includes comprehensive VT100/VT220/VT320/VT420 compatibility with modern protocol support:

### Core Features ✅

1. **Alt Screen Buffer** - Fully implemented with modes 47, 1047, 1049
2. **Tab Stops** - Complete tab stop management (HTS, TBC, CHT, CBT)
3. **Line Wrapping** - Auto-wrap mode (DECAWM) with delayed wrap
4. **Hyperlinks** - Full OSC 8 hyperlink support with deduplication
5. **Sixel Graphics** - Complete Sixel implementation with half-block rendering
6. **Wide Character Support** - Unicode, emoji, and CJK characters

### Modern Protocols ✅

1. **Mouse Tracking** - All modes (Normal, Button, Any) and encodings (SGR, UTF-8, URXVT)
2. **Bracketed Paste** - Mode 2004 for safe paste handling
3. **Synchronized Updates** - Mode 2026 for flicker-free rendering
4. **Kitty Keyboard Protocol** - Enhanced keyboard reporting with flag management
5. **Shell Integration** - OSC 133 for prompt/command/output markers
6. **Clipboard** - OSC 52 read/write with security controls

### VT Compatibility ✅

- VT100/VT220/VT320 - Complete compatibility
- VT420 - Rectangle operations (DECFRA, DECCRA, DECSERA)
- Left/Right Margins - DECLRMM/DECSLRM support
- Cursor Styles - DECSCUSR with all styles
- Device Queries - DA, DSR, CPR, DECRQM

## Future Enhancements

### Potential Improvements

1. **Unicode Normalization**: Proper grapheme cluster handling for combining marks
2. **Performance**: SIMD optimizations for bulk cell operations
3. **Character Sets**: G0/G1/G2/G3 selection (low priority - UTF-8 handles most cases)

### API Enhancements

1. **Cell Iterators**: Efficient row/region iteration without copying
2. **Diff API**: Change detection for efficient incremental rendering
3. **Event Callbacks**: Async callbacks for title change, resize, bell, etc.
4. **Async Support**: Fully async Python API for non-blocking operation

## Screenshot Module

### Architecture (`src/screenshot/`)

The screenshot module provides high-quality rendering of terminal content to various image formats:

#### Components

1. **Configuration** (`config.rs`)
   - **Purpose**: Screenshot configuration and format options
   - **Features**:
     - Image format selection (PNG, JPEG, BMP, SVG)
     - Font size and padding configuration
     - Sixel rendering mode options
     - Quality settings for lossy formats

2. **Font Cache** (`font_cache.rs`)
   - **Library**: Swash (pure Rust font library)
   - **Purpose**: Loads and caches font glyphs for efficient rendering
   - **Features**:
     - Embedded JetBrains Mono font (no external dependencies)
     - Embedded Noto Emoji font for emoji support
     - Automatic emoji font fallback (Apple Color Emoji, Segoe UI Emoji)
     - Color emoji rendering with RGBA output
     - Glyph caching for performance (by character, size, bold, italic)
     - Glyph-by-ID rendering for shaped text
   - **Embedded Fonts**: `JetBrainsMono-Regular.ttf`, `NotoEmoji-Regular.ttf`

3. **Text Shaper** (`shaper.rs`)
   - **Library**: Swash (pure Rust text shaping and font rendering)
   - **Purpose**: Handles complex text rendering with ligatures and multi-codepoint sequences
   - **Features**:
     - Flag emoji support via Regional Indicator ligatures (🇺🇸 🇨🇳 🇯🇵)
     - Multi-font support (Regular, Emoji, CJK) with automatic selection
     - Positioned glyph output with advance/offset information
     - Font run segmentation for mixed-script text
     - Pure Rust implementation (no C dependencies, no HarfBuzz)

4. **Renderer** (`renderer.rs`)
   - **Purpose**: Converts terminal grid to image pixels
   - **Features**:
     - Hybrid rendering: character-based (fast) + line-based shaping (complex emoji)
     - Regional Indicator detection for automatic text shaping
     - Full text attribute support (bold, italic, underline styles, colors)
     - Cursor rendering (block, underline, bar styles)
     - Sixel graphics rendering (pixels and half-block modes)
     - Alpha blending for smooth text and graphics
     - Pure Rust rendering pipeline (no C dependencies)

5. **Utilities** (`utils.rs`)
   - **Purpose**: Helper functions for screenshot rendering
   - **Features**:
     - Color conversion and blending utilities
     - Text measurement and positioning helpers
     - Regional Indicator detection for emoji flags

6. **Error Handling** (`error.rs`)
   - **Purpose**: Screenshot-specific error types
   - **Features**:
     - Comprehensive error variants for font, rendering, and encoding failures
     - Integration with standard error handling

7. **Format Support** (`formats/`)
   - **Modules**: `mod.rs`, `png.rs`, `jpeg.rs`, `bmp.rs`, `svg.rs`
   - **Raster formats**: PNG, JPEG, BMP (via `image` crate)
   - **Vector format**: SVG (custom implementation for scalable text)

### Font Rendering Pipeline

```mermaid
graph TD
    A[Character Input]
    B[FontCache::get_glyph]
    C{Check cache?}
    D[Return cached glyph]
    E[Emoji detection<br/>Unicode range]
    F[Try main font<br/>JetBrains Mono]
    G{Empty or emoji?}
    H[Try emoji font]
    I[Swash rendering<br/>- Set pixel size<br/>- Render glyph<br/>- Handle color emoji]
    J[Cache result]
    K[Return CachedGlyph]

    A --> B
    B --> C
    C -->|Hit| D
    C -->|Miss| E
    E --> F
    F --> G
    G -->|Yes| H
    G -->|No| I
    H --> I
    I --> J
    J --> K

    style A fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style B fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style C fill:#ff6f00,stroke:#ffa726,stroke-width:2px,color:#ffffff
    style D fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
    style E fill:#37474f,stroke:#78909c,stroke-width:2px,color:#ffffff
    style F fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style G fill:#ff6f00,stroke:#ffa726,stroke-width:2px,color:#ffffff
    style H fill:#1a237e,stroke:#3f51b5,stroke-width:2px,color:#ffffff
    style I fill:#880e4f,stroke:#c2185b,stroke-width:2px,color:#ffffff
    style J fill:#37474f,stroke:#78909c,stroke-width:2px,color:#ffffff
    style K fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
```

### Bitmap Font Handling

Color emoji fonts (like NotoColorEmoji) are bitmap-only fonts that:
- Cannot be scaled to arbitrary sizes
- Have fixed sizes (typically 32, 64, 72, 96, 109, 128, 136 pixels)
- Require special handling during size selection

The implementation automatically:
1. Attempts requested size
2. Falls back to closest available fixed size
3. Renders with swash's color emoji support
4. Outputs RGBA for consistent image processing

### Rendering Pipeline

```mermaid
graph TD
    A[Terminal.screenshot]
    B[Create Renderer<br/>FontCache + TextShaper + Config]
    C[For each grid row]
    D{Contains Regional<br/>Indicators?}
    E[Swash text shaping<br/>- Extract line text<br/>- Shape with TextShaper<br/>- Render positioned glyphs]
    F[Fast character rendering<br/>For each cell:<br/>- Resolve colors<br/>- Render background<br/>- Render character<br/>- Render decorations]
    G[Render Sixel graphics]
    H[Render cursor<br/>if visible]
    I[Encode to format<br/>PNG/JPEG/BMP/SVG]
    J[Return bytes]

    A --> B
    B --> C
    C --> D
    D -->|Yes| E
    D -->|No| F
    E --> G
    F --> G
    G --> H
    H --> I
    I --> J

    style A fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style B fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style C fill:#37474f,stroke:#78909c,stroke-width:2px,color:#ffffff
    style D fill:#ff6f00,stroke:#ffa726,stroke-width:2px,color:#ffffff
    style E fill:#880e4f,stroke:#c2185b,stroke-width:2px,color:#ffffff
    style F fill:#1a237e,stroke:#3f51b5,stroke-width:2px,color:#ffffff
    style G fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style H fill:#2e7d32,stroke:#66bb6a,stroke-width:2px,color:#ffffff
    style I fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style J fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
```

### Text Shaping Pipeline (Flag Emoji)

```mermaid
graph TD
    A[Line with Regional<br/>Indicators detected]
    B[Grid::row_text<br/>Extract line as string]
    C[TextShaper::shape_line]
    D[Split into font runs<br/>Regular/Emoji/CJK]
    E[For each run:<br/>- Select font<br/>- Create shape context<br/>- Shape with swash<br/>- Extract glyph IDs + positions]
    F[For each shaped glyph:<br/>- FontCache::get_glyph_by_id<br/>- Apply x_offset, y_offset<br/>- Render with alpha blending]
    G[Complete line rendered<br/>with proper ligatures]

    A --> B
    B --> C
    C --> D
    D --> E
    E --> F
    F --> G

    style A fill:#ff6f00,stroke:#ffa726,stroke-width:2px,color:#ffffff
    style B fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style C fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style D fill:#1a237e,stroke:#3f51b5,stroke-width:2px,color:#ffffff
    style E fill:#880e4f,stroke:#c2185b,stroke-width:2px,color:#ffffff
    style F fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style G fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
```

## Dependencies

### Rust

**Core dependencies:**
- `pyo3`: 0.27.1 - Python bindings
- `vte`: 0.15.0 - ANSI parser
- `unicode-width`: 0.2.2 - Character width calculation
- `portable-pty`: 0.9.0 - PTY support
- `base64`: 0.22.1 - Base64 encoding/decoding
- `bitflags`: 2.10.0 - Bit flag management

**Screenshot/rendering support:**
- `image`: 0.25.8 - Image encoding/decoding (PNG, JPEG, BMP)
- `swash`: 0.2.6 - Pure Rust font rendering and text shaping with color emoji support

**Development dependencies:**
- `proptest`: 1.9.0 - Property-based testing framework

**Platform-specific:**
- `libc`: 0.2.177 - Unix system calls (Unix only)

### Python

**Build and development tools:**
- `maturin`: >=1.10.1 - Build system for PyO3 bindings
- `uv`: Latest - Fast Python package installer and resolver

**Testing:**
- `pytest`: >=9.0.1 - Testing framework
- `pytest-timeout`: >=2.4.0 - Test timeout protection

**Code quality:**
- `ruff`: >=0.14.5 - Linting and formatting
- `pyright`: >=1.1.407 - Static type checking
- `pre-commit`: >=4.4.0 - Git hook management

**Python version requirements:** 3.12, 3.13, or 3.14

> **Note**: This is a core library. For a full-featured TUI application built on this library, see the sister project [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust) ([PyPI](https://pypi.org/project/par-term-emu-tui-rust/)), which uses the Textual framework.

## Build Process

### PyO3 Feature Configuration

The project uses conditional PyO3 feature compilation to support both production builds and testing:

**Cargo.toml features:**
```toml
[dependencies]
pyo3 = "0.27.1"

[dev-dependencies]
pyo3 = { version = "0.27.1", features = ["auto-initialize"] }

[features]
default = ["pyo3/extension-module"]
```

**Build commands:**
- **Development build:** `maturin develop --release` (uses `extension-module` feature)
- **Running Rust tests:** `cargo test --lib --no-default-features --features pyo3/auto-initialize`
- **Production wheels:** `maturin build --release` (uses default features with `extension-module`)

> **⚠️ Important:** Never run `cargo build` directly for PyO3 modules. Always use `maturin develop` or the `make dev` target to ensure proper Python integration.

**Why these features:**
- **`extension-module`:** Tells linker NOT to link against libpython (correct for Python extensions)
- **`auto-initialize`:** Initializes Python interpreter for Rust tests (required for `cargo test`)
- **Default feature:** Enables `extension-module` automatically for production builds
- **Test override:** Uses `--no-default-features` to disable `extension-module` during testing

### Build Flow

```mermaid
graph TD
    A[Source Code .rs]
    B[Rust Compiler]
    C[Shared Library<br/>.so / .dll / .dylib]
    D[Maturin Packaging]
    E[Python Wheel .whl]
    F[Installation]

    A --> B
    B --> C
    C --> D
    D --> E
    E --> F

    style A fill:#4a148c,stroke:#9c27b0,stroke-width:2px,color:#ffffff
    style B fill:#e65100,stroke:#ff9800,stroke-width:3px,color:#ffffff
    style C fill:#0d47a1,stroke:#2196f3,stroke-width:2px,color:#ffffff
    style D fill:#1a237e,stroke:#3f51b5,stroke-width:2px,color:#ffffff
    style E fill:#2e7d32,stroke:#66bb6a,stroke-width:2px,color:#ffffff
    style F fill:#1b5e20,stroke:#4caf50,stroke-width:2px,color:#ffffff
```

## Continuous Integration

### CI/CD Pipeline

The project uses GitHub Actions (`.github/workflows/ci.yml`) with three parallel jobs:

#### Test Job
- **Platforms:** Ubuntu, macOS, Windows
- **Python versions:** 3.12, 3.13, 3.14 (matrix: 9 combinations)
- **Timeout:** 15 minutes per job
- **Steps:**
  1. **Rust tests:** `cargo test --lib --no-default-features --features pyo3/auto-initialize`
  2. **Python tests:** `pytest tests/ -v --timeout=5 --timeout-method=thread`
  3. **PTY tests excluded in CI:**
     - `test_pty.py` - Hangs in automated environments
     - `test_ioctl_size.py` - Requires real PTY
     - `test_pty_resize_sigwinch.py` - Signal handling issues in CI
     - `test_nested_shell_resize.py` - Complex PTY interactions

#### Lint Job
- **Platform:** Ubuntu only
- **Python version:** 3.14
- **Timeout:** 15 minutes
- **Checks:**
  - Rust formatting: `cargo fmt -- --check`
  - Rust clippy: `cargo clippy --all-targets --all-features -- -D warnings`
  - Python formatting: `ruff format --check`
  - Python linting: `ruff check`
  - Python type checking: `pyright`

#### Build Job
- **Platforms:** Ubuntu, macOS, Windows
- **Python version:** 3.14
- **Timeout:** 15 minutes
- **Output:** Platform-specific wheels uploaded as artifacts
- **Command:** `maturin build --release`

### Running Checks Locally

```bash
# Run all checks with auto-fix
make checkall

# Individual checks
make test-rust    # Rust unit tests
make test-python  # Python integration tests
cargo fmt         # Format Rust code
cargo clippy      # Lint Rust code
uv run ruff format .  # Format Python code
uv run ruff check .   # Lint Python code
uv run pyright .      # Type check Python code
```

### Pre-commit Hooks

The project uses `pre-commit` hooks to enforce quality standards. Install with:

```bash
make pre-commit-install  # or: uv run pre-commit install
```

**Hooks enabled:**
- Trailing whitespace removal
- End-of-file fixing
- YAML/TOML syntax checking
- Large file detection
- Rust formatting (`cargo fmt`)
- Rust linting (`cargo clippy`)
- Rust tests (`cargo test --lib --no-default-features --features pyo3/auto-initialize`)
- Python formatting (`ruff format`)
- Python linting (`ruff check --fix`)
- Python type checking (`pyright`)
- Python tests (`pytest`)

## Debugging

### Rust Side

```bash
# Enable debug logging
RUST_LOG=debug cargo test

# Use rust-lldb/gdb
rust-lldb target/debug/test_binary
```

### Python Side

```python
# Inspect terminal state
print(repr(term))
print(term.content())
print(term.cursor_position())

# Check individual cells
for row in range(term.size()[1]):
    for col in range(term.size()[0]):
        char = term.get_char(col, row)
        print(f"({col},{row}): {char}")
```

## Contributing

When contributing, please:

1. Add tests for new features
2. Update documentation
3. Follow Rust style guidelines (`cargo fmt`)
4. Pass clippy lints (`cargo clippy`)
5. Ensure Python API remains intuitive

## References

- [VTE Crate Documentation](https://docs.rs/vte/) - ANSI parser library
- [PyO3 Guide](https://pyo3.rs/) - Rust-Python bindings
- [xterm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - Comprehensive reference
- [ANSI Escape Sequences](https://en.wikipedia.org/wiki/ANSI_escape_code) - Wikipedia overview
- [VT100 Reference](https://vt100.net/) - Historical VT100 documentation

## See Also

- [VT_FEATURE_PARITY.md](VT_FEATURE_PARITY.md) - Complete VT feature support matrix
- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced features guide
- [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) - Terminal configuration reference
- [BUILDING.md](BUILDING.md) - Build and installation instructions
- [SECURITY.md](SECURITY.md) - Security considerations for PTY usage
- [README.md](../README.md) - Project overview and API reference
