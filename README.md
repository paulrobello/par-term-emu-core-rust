# Par Term Emu Core Rust

[![PyPI](https://img.shields.io/pypi/v/par_term_emu_core_rust)](https://pypi.org/project/par_term_emu_core_rust/)
[![PyPI - Python Version](https://img.shields.io/pypi/pyversions/par_term_emu_core_rust.svg)](https://pypi.org/project/par_term_emu_core_rust/)
![Runs on Linux | MacOS | Windows](https://img.shields.io/badge/runs%20on-Linux%20%7C%20MacOS%20%7C%20Windows-blue)
![Arch x86-64 | ARM | AppleSilicon](https://img.shields.io/badge/arch-x86--64%20%7C%20ARM%20%7C%20AppleSilicon-blue)
![PyPI - Downloads](https://img.shields.io/pypi/dm/par_term_emu_core_rust)
![PyPI - License](https://img.shields.io/pypi/l/par_term_emu_core_rust)

## Description

A comprehensive terminal emulator library written in Rust with Python bindings for Python 3.12+. Provides VT100/VT220/VT320/VT420 compatibility with PTY support, matching iTerm2's feature set.

[!["Buy Me A Coffee"](https://www.buymeacoffee.com/assets/img/custom_images/orange_img.png)](https://buymeacoffee.com/probello3)

## Table of Contents

- [Description](#description)
- [Technology](#technology)
- [Prerequisites](#prerequisites)
- [Features](#features)
- [Installation](#installation)
  - [From Source](#from-source)
  - [Building a Wheel](#building-a-wheel)
  - [Terminfo Installation (Optional)](#terminfo-installation-optional)
  - [Shell Integration (Optional but Recommended)](#shell-integration-optional-but-recommended)
- [Quick Start](#quick-start)
  - [PTY Quick Start](#pty-quick-start)
- [Usage Examples](#usage-examples)
- [API Reference](#api-reference)
- [Supported ANSI/VT Sequences](#supported-ansivt-sequences)
- [Examples](#examples)
- [TUI Demo Application](#tui-demo-application)
- [Security](#security)
- [Running Tests](#running-tests)
- [Performance](#performance)
- [Architecture](#architecture)
- [Contributing](#contributing)
- [License](#license)
- [Author](#author)

## Technology

- Rust (1.75+)
- Python (3.12+)
- PyO3 for Python bindings
- VTE for ANSI parsing
- portable-pty for cross-platform PTY support

## Prerequisites

- Rust 1.75 or higher
- Python 3.12 or higher
- uv package manager
- maturin (for building Python wheels)

## Features

### Core VT Compatibility (iTerm2 Feature Parity)

- **VT100/VT220/VT320/VT420 Support**: Comprehensive terminal emulation matching iTerm2's capabilities
- **Rich Color Support**:
  - Basic 16 ANSI colors
  - 256-color palette
  - 24-bit RGB (true color)
- **Text Attributes**: Bold, italic, underline (with styles: straight, double, curly, dotted, dashed), strikethrough, blink, reverse, dim, and hidden
- **Advanced Cursor Control**: Full VT100 cursor movement and positioning
- **Line/Character Editing**: VT220 insert/delete operations (IL, DL, ICH, DCH, ECH)
- **Rectangle Operations**: VT420 fill/copy/erase/modify rectangular regions (DECFRA, DECCRA, DECSERA, DECERA, DECCARA, DECRARA, DECRQCRA, DECSACE)
- **Scrolling Regions**: DECSTBM for restricted scrolling areas
- **Tab Stops**: Configurable tab stops (HTS, TBC, CHT, CBT)
- **Terminal Modes**:
  - Application cursor keys (DECCKM)
  - Origin mode (DECOM)
  - Auto wrap mode (DECAWM)
  - Multiple alternate screen variants (47, 1047, 1049)
- **Mouse Support**: Multiple mouse tracking modes (X10, Normal, Button, Any) and encodings (Default, UTF-8, SGR, URXVT)
- **Modern Features**:
  - Alternate screen buffer
  - Bracketed paste mode
  - Focus tracking
  - OSC 8 hyperlinks (full support - clickable in TUI)
  - OSC 52 clipboard operations (copy/paste over SSH without X11)
  - OSC 9 / OSC 777 notifications (desktop-style alerts)
  - Shell integration (OSC 133)
  - Sixel graphics (DCS format with half-block rendering)
  - Kitty Keyboard Protocol (progressive enhancement for keyboard handling)
- **Scrollback Buffer**: Configurable history of scrolled lines
- **Terminal Resizing**: Dynamic size adjustment
- **Unicode Support**: Full Unicode including emoji and wide characters
- **High Performance**: Written in Rust for speed and safety
- **Python Integration**: Easy-to-use Python API via PyO3

### PTY Support

- **Interactive Shell Sessions**: Spawn and control shell processes
- **Bidirectional I/O**: Send input and receive output from running processes
- **Process Management**: Start, stop, and monitor child processes
- **Dynamic Resizing**: Resize terminal and send SIGWINCH to child process
- **Environment Control**: Custom environment variables and working directory
- **Event Loop Integration**: Non-blocking update detection for efficient event loops
- **Context Manager Support**: Automatic cleanup with Python `with` statements
- **Cross-Platform**: Works on Linux, macOS, and Windows via portable-pty

### Utility Functions

- **Text Extraction**: Smart word/URL detection, line unwrapping, selection boundaries
- **Content Search**: Find text with case-sensitive/insensitive matching, "find next" support
- **Buffer Statistics**: Get detailed metrics (memory usage, cell counts, graphics count)
- **Static Utilities**: Strip ANSI codes, measure text width, parse colors from strings

### Screenshot Support

- **Multiple Formats**: PNG, JPEG, BMP, **SVG** (vector graphics!), and **HTML**
- **Embedded Font**: JetBrains Mono bundled - no font installation required!
- **Programming Ligatures**: Supports =>, !=, >=, and other code ligatures
- **Box Drawing**: Perfect rendering of box drawing characters (┌─┐│└┘)
- **High-Quality Rendering**: True font rendering with antialiasing for raster formats
- **SVG Vector Output**: Infinitely scalable screenshots with selectable text
- **HTML Output**: Styled HTML with full color support and text attributes (includes full document or content-only mode)
- **Color Emoji Support**: Full color emoji rendering with automatic font fallback (NotoColorEmoji, Apple Color Emoji, Segoe UI Emoji)
- **Flag Emoji Support**: Proper rendering of flag emojis (🇺🇸 🇨🇳 🇯🇵) via text shaping
- **Cursor Rendering**: Capture cursor position with 3 styles (Block, Underline, Bar) and custom colors
- **Sixel Graphics**: Inline graphics rendering with full alpha blending support
- **Configurable**: Custom fonts, sizes, padding, and quality settings
- **Full Styling**: Preserves all colors, text attributes, and decorations
- **Wide Character Support**: Renders CJK characters and emoji correctly
- **Python & Rust APIs**: Easy-to-use interfaces for both languages
- **TUI Integration**: Built-in `Ctrl+Shift+S` hotkey in the TUI application

## Installation

### From Source

Requires Rust 1.75+ and Python 3.12+:

```bash
# Install maturin (build tool) using uv
uv tool install maturin

# Build and install the package
maturin develop --release
```

### Building a Wheel

```bash
maturin build --release
uv pip install target/wheels/par_term_emu_core_rust-*.whl
```

### Terminfo Installation (Optional)

For optimal terminal compatibility, you can install the par-term terminfo definition. This allows applications to fully utilize all terminal capabilities (true color, Sixel graphics, etc.):

```bash
# Install for current user
./terminfo/install.sh

# Or install system-wide (requires sudo)
sudo ./terminfo/install.sh --system

# Then use the terminal type
export TERM=par-term
export COLORTERM=truecolor
```

See [terminfo/README.md](terminfo/README.md) for detailed information.

**Note**: If not installed, the terminal defaults to `xterm-256color` compatibility mode, which works well with most applications.

### Shell Integration (Optional but Recommended)

Shell integration enhances your terminal experience with semantic prompt markers. This enables features like prompt navigation, command status tracking, and smart selection.

```bash
# Install for your current shell (auto-detects bash/zsh/fish)
cd shell_integration
./install.sh

# Or install for all available shells
./install.sh --all

# Or install for a specific shell
./install.sh bash   # or zsh, or fish
```

After installation, restart your shell or run:
```bash
source ~/.par_term_emu_core_rust_shell_integration.bash  # for bash
source ~/.par_term_emu_core_rust_shell_integration.zsh   # for zsh
source ~/.par_term_emu_core_rust_shell_integration.fish  # for fish
```

See [shell_integration/README.md](shell_integration/README.md) for detailed information, advanced usage, and customization options.

## Quick Start

```python
from par_term_emu_core_rust import Terminal

# Create a terminal with 80 columns and 24 rows
term = Terminal(80, 24)

# Process some text with ANSI codes
term.process_str("Hello, \x1b[31mWorld\x1b[0m!\n")
term.process_str("\x1b[1;32mBold green text\x1b[0m\n")

# Get the content
print(term.content())

# Check cursor position
col, row = term.cursor_position()
print(f"Cursor at: ({col}, {row})")
```

### PTY Quick Start

```python
from par_term_emu_core_rust import PtyTerminal
import time

# Create a terminal and spawn a shell
with PtyTerminal(80, 24) as term:
    term.spawn_shell()

    # Send a command
    term.write_str("echo 'Hello from shell!'\n")
    time.sleep(0.2)

    # Get output
    print(term.content())

    # Resize terminal
    term.resize(100, 30)

    # Exit shell
    term.write_str("exit\n")
# Automatic cleanup on exit
```

## Usage Examples

### Basic Text Processing

```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

# Write plain text
term.process_str("Hello, Terminal!\n")

# Write bytes
term.process(b"Binary data\n")

# Get content
content = term.content()
print(content)
```

### Colors and Styling

```python
term = Terminal(80, 24)

# Basic colors (30-37 for foreground, 40-47 for background)
term.process_str("\x1b[31mRed text\x1b[0m\n")
term.process_str("\x1b[42mGreen background\x1b[0m\n")

# Bright colors (90-97, 100-107)
term.process_str("\x1b[91mBright red\x1b[0m\n")

# 256-color palette
term.process_str("\x1b[38;5;208mOrange text\x1b[0m\n")

# 24-bit RGB colors
term.process_str("\x1b[38;2;255;128;0mRGB orange\x1b[0m\n")

# Text attributes
term.process_str("\x1b[1mBold\x1b[0m ")
term.process_str("\x1b[3mItalic\x1b[0m ")
term.process_str("\x1b[4mUnderline\x1b[0m\n")
```

### Cursor Control

```python
term = Terminal(80, 24)

# Position cursor at row 5, column 10 (1-indexed)
term.process_str("\x1b[5;10HHello")

# Move cursor
term.process_str("\x1b[2A")    # Up 2 rows
term.process_str("\x1b[3B")    # Down 3 rows
term.process_str("\x1b[4C")    # Forward 4 columns
term.process_str("\x1b[5D")    # Back 5 columns

# Hide/show cursor
term.process_str("\x1b[?25l")  # Hide
term.process_str("\x1b[?25h")  # Show

# Check cursor state
visible = term.cursor_visible()
col, row = term.cursor_position()
```

### Screen Manipulation

```python
term = Terminal(80, 24)

term.process_str("Some content\n")

# Clear operations
term.process_str("\x1b[2J")    # Clear entire screen
term.process_str("\x1b[K")     # Clear from cursor to end of line
term.process_str("\x1b[1K")    # Clear from beginning of line to cursor
term.process_str("\x1b[2K")    # Clear entire line

# Scroll
term.process_str("\x1b[5S")    # Scroll up 5 lines
term.process_str("\x1b[5T")    # Scroll down 5 lines
```

### Scrollback Buffer

```python
term = Terminal(40, 10, scrollback=1000)

# Write more lines than terminal height
for i in range(20):
    term.process_str(f"Line {i}\n")

# Get scrollback
scrollback = term.scrollback()
print(f"Scrollback has {len(scrollback)} lines")

for line in scrollback:
    print(f"Scrolled: {line}")
```

### Terminal Resizing

```python
term = Terminal(80, 24)

# Write some content
term.process_str("Content before resize\n")

# Resize terminal
term.resize(100, 30)

# Content is preserved
print(term.size())  # (100, 30)
```

### Cell Inspection

```python
term = Terminal(80, 24)
term.process_str("\x1b[1;31mRed bold text\x1b[0m")

# Get character at position
char = term.get_char(0, 0)
print(f"Character: {char}")

# Get colors
fg_color = term.get_fg_color(0, 0)
bg_color = term.get_bg_color(0, 0)
if fg_color:
    r, g, b = fg_color
    print(f"Foreground: RGB({r}, {g}, {b})")

# Get attributes
attrs = term.get_attributes(0, 0)
if attrs:
    print(f"Bold: {attrs.bold}")
    print(f"Italic: {attrs.italic}")
    print(f"Underline: {attrs.underline}")
```

### Buffer Export

```python
term = Terminal(80, 24, scrollback=1000)

# Write multiple lines with styling
for i in range(50):
    term.process_str(f"\x1b[1;{31 + i % 7}mLine {i}\x1b[0m\n")

# Export as plain text (no styling)
plain_text = term.export_text()
with open("session.txt", "w") as f:
    f.write(plain_text)

# Export with ANSI styling preserved
styled_text = term.export_styled()
with open("session.ansi", "w") as f:
    f.write(styled_text)

# Export as HTML with full styling
html_output = term.export_html(include_styles=True)
with open("session.html", "w") as f:
    f.write(html_output)  # Full HTML document with styles

# Export HTML content only (for embedding)
html_content = term.export_html(include_styles=False)
# Returns just the styled <span> elements, no <html>/<head>/<body>

# Also works with PTY sessions
with PtyTerminal(80, 24) as pty:
    pty.spawn_shell()
    pty.write_str("ls -la\n")
    time.sleep(0.5)

    # Save entire session (scrollback + screen)
    session_log = pty.export_styled()
    with open("pty_session.log", "w") as f:
        f.write(session_log)
```

### Screenshots

Take high-quality screenshots of terminal output with true font rendering:

```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

# Add some colorful content
term.process_str("\x1b[1;31mHello, World!\x1b[0m\n")
term.process_str("\x1b[32m✓ Success\x1b[0m\n")

# Take screenshot as PNG bytes (default)
png_bytes = term.screenshot()

# Save to file (format auto-detected from extension)
term.screenshot_to_file("output.png")

# Or explicitly specify format
term.screenshot_to_file("output.jpg", format="jpeg", quality=90)

# SVG format for infinitely scalable vector graphics
term.screenshot_to_file("output.svg", format="svg")  # Selectable text!

# HTML format for styled terminal output (embeddable or standalone)
term.screenshot_to_file("output.html", format="html")  # Full HTML document

# Custom configuration
term.screenshot_to_file(
    "output.png",
    font_size=16.0,           # Larger text
    padding=20,               # More padding around edges
    include_scrollback=True   # Include scrollback history
)

# Capture scrolled view (e.g., 10 lines back from current position)
term.screenshot_to_file(
    "output.png",
    scrollback_offset=10      # Capture from 10 lines back
)

# Customize theme colors
term.screenshot_to_file(
    "output.png",
    link_color=(0, 123, 255),     # Custom hyperlink color
    bold_color=(255, 255, 0),     # Custom bold text color
    use_bold_color=True           # Apply custom bold color
)

# Specify custom font (for raster formats)
term.screenshot_to_file(
    "output.png",
    font_path="/path/to/font.ttf"
)
```

#### Supported Formats

- **PNG**: Lossless, best for text and screenshots (default)
- **JPEG**: Smaller file size, configurable quality (1-100)
- **BMP**: Uncompressed, large file size
- **SVG**: Vector format with infinitely scalable, selectable text - perfect for documentation!
- **HTML**: Styled HTML output with full color support and text attributes - embeddable or standalone document

#### TUI Integration

> **Note**: TUI integration features are available in the sister project [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust), which includes screenshot hotkeys and other interactive features.

### Text Extraction & Smart Selection

Extract words, URLs, and lines with intelligent boundary detection:

```python
term = Terminal(80, 24)
term.process_str("Visit https://example.com for more info\n")
term.process_str("Use snake_case or kebab-case naming\n")
term.process_str("Function call: foo(bar, {x: 1, y: \"hello\"})\n")

# Extract word at cursor position
word = term.get_word_at(5, 0)  # "https://example.com"

# Detect and extract URLs
url = term.get_url_at(10, 0)  # "https://example.com"

# Get word boundaries for double-click selection
bounds = term.select_word(5, 1)
if bounds:
    (start_col, start_row), (end_col, end_row) = bounds
    print(f"Word spans from ({start_col}, {start_row}) to ({end_col}, {end_row})")

# Custom word characters (include underscores)
word = term.get_word_at(4, 1, word_chars="_-")  # "snake_case"

# Get full logical line (following wraps)
full_line = term.get_line_unwrapped(0)

# Find matching bracket/parenthesis
term.process_str("if (condition) { action(); }\n")
match_pos = term.find_matching_bracket(3, 3)  # Click on opening '('
if match_pos:
    col, row = match_pos
    print(f"Matching bracket at ({col}, {row})")  # Position of closing ')'

# Select content within delimiters (semantic selection)
term.process_str('message = "Hello, World!"\n')
content = term.select_semantic_region(15, 4, '"')  # Click inside quotes
print(content)  # "Hello, World!"

# Works with multiple delimiter types
term.process_str("data = {key: 'value', num: [1, 2, 3]}\n")
content = term.select_semantic_region(10, 5, "{}[]'\"")  # Click inside braces
print(content)  # "key: 'value', num: [1, 2, 3]"
```

### Content Search

Search terminal content with case-sensitive or case-insensitive matching:

```python
term = Terminal(80, 24)
for i in range(20):
    term.process_str(f"Line {i}: Error in module_{i}\n")

# Find all occurrences
matches = term.find_text("Error", case_sensitive=True)
print(f"Found {len(matches)} matches: {matches}")  # [(8, 0), (8, 1), ...]

# Case-insensitive search
matches = term.find_text("error", case_sensitive=False)

# Find next occurrence from position
next_match = term.find_next("Error", from_col=0, from_row=5)
if next_match:
    col, row = next_match
    print(f"Next match at ({col}, {row})")

# Implement "find next" button
current_pos = (0, 0)
while True:
    match = term.find_next("pattern", current_pos[0], current_pos[1])
    if not match:
        break
    print(f"Match at {match}")
    current_pos = match
```

### Buffer Statistics & Analysis

Get comprehensive statistics about terminal content:

```python
term = Terminal(80, 24, scrollback=10000)

# Get detailed statistics
stats = term.get_stats()
print(f"Dimensions: {stats['cols']}x{stats['rows']}")
print(f"Scrollback: {stats['scrollback_lines']}/{stats['total_cells']} cells")
print(f"Non-empty lines: {stats['non_whitespace_lines']}")
print(f"Graphics: {stats['graphics_count']}")
print(f"Memory: ~{stats['estimated_memory_bytes']} bytes")

# Count content lines (excluding empty lines)
content_lines = term.count_non_whitespace_lines()
print(f"Lines with content: {content_lines}")

# Check scrollback usage
used, capacity = term.get_scrollback_usage()
print(f"Scrollback: {used}/{capacity} lines ({used/capacity*100:.1f}%)")
```

### Static Utility Methods

Use standalone utility functions for text processing:

```python
# Strip ANSI codes from text
colored = "\x1b[31mRed\x1b[0m text"
clean = Terminal.strip_ansi(colored)
print(clean)  # "Red text"

# Measure display width (accounts for wide chars)
text = "Hello 世界"
width = Terminal.measure_text_width(text)
print(f"Display width: {width} columns")  # 11 (5 + 1 + 2 + 2 + 1)

# Handle ANSI codes in width calculation
text_with_ansi = "\x1b[31mHello\x1b[0m"
width = Terminal.measure_text_width(text_with_ansi)
print(f"Width: {width}")  # 5 (ANSI codes don't count)

# Parse colors from various formats
rgb = Terminal.parse_color("#FF5733")  # (255, 87, 51)
rgb = Terminal.parse_color("rgb(255, 87, 51)")  # (255, 87, 51)
rgb = Terminal.parse_color("red")  # (180, 60, 42) - iTerm2 red
rgb = Terminal.parse_color("invalid")  # None
```

## API Reference

### Terminal Class

#### Constructor

```python
Terminal(cols: int, rows: int, scrollback: int = 10000)
```

Create a new terminal with specified dimensions.

- `cols`: Number of columns (width)
- `rows`: Number of rows (height)
- `scrollback`: Maximum number of scrollback lines (default: 10000)

#### Methods

- `process(data: bytes)`: Process byte data (can contain ANSI sequences)
- `process_str(text: str)`: Process a string (convenience method)
- `content() -> str`: Get terminal content as a string
- `size() -> tuple[int, int]`: Get terminal dimensions (cols, rows)
- `resize(cols: int, rows: int)`: Resize the terminal
- `reset()`: Reset terminal to default state
- `title() -> str`: Get terminal title
- `cursor_position() -> tuple[int, int]`: Get cursor position (col, row)
- `cursor_visible() -> bool`: Check if cursor is visible
- `keyboard_flags() -> int`: Get current Kitty Keyboard Protocol flags
- `set_keyboard_flags(flags: int, mode: int = 1)`: Set Kitty Keyboard Protocol flags (mode: 0=disable, 1=set, 2=lock)
- `query_keyboard_flags()`: Query keyboard flags (response in drain_responses())
- `push_keyboard_flags(flags: int)`: Push flags to stack and set new flags
- `pop_keyboard_flags(count: int = 1)`: Pop flags from stack
- `clipboard() -> str | None`: Get clipboard content (OSC 52)
- `set_clipboard(content: str | None)`: Set clipboard content programmatically
- `allow_clipboard_read() -> bool`: Check if clipboard read is allowed
- `set_allow_clipboard_read(allow: bool)`: Set clipboard read permission (security flag)
- `scrollback() -> list[str]`: Get scrollback buffer as list of strings
- `scrollback_len() -> int`: Get number of scrollback lines
- `get_line(row: int) -> str | None`: Get a specific line
- `get_line_cells(row: int) -> list | None`: Get cells for a specific line with full metadata
- `get_char(col: int, row: int) -> str | None`: Get character at position
- `get_fg_color(col: int, row: int) -> tuple[int, int, int] | None`: Get foreground color (RGB)
- `get_bg_color(col: int, row: int) -> tuple[int, int, int] | None`: Get background color (RGB)
- `get_underline_color(col: int, row: int) -> tuple[int, int, int] | None`: Get underline color (RGB)
- `get_attributes(col: int, row: int) -> Attributes | None`: Get text attributes
- `get_hyperlink(col: int, row: int) -> str | None`: Get hyperlink URL at position (OSC 8)
- `is_line_wrapped(row: int) -> bool`: Check if line is wrapped from previous line
- `is_alt_screen_active() -> bool`: Check if alternate screen buffer is active
- `bracketed_paste() -> bool`: Check if bracketed paste mode is enabled
- `focus_tracking() -> bool`: Check if focus tracking mode is enabled
- `mouse_mode() -> str`: Get current mouse tracking mode
- `insert_mode() -> bool`: Check if insert mode is enabled
- `line_feed_new_line_mode() -> bool`: Check if line feed/new line mode is enabled
- `synchronized_updates() -> bool`: Check if synchronized updates mode is enabled (DEC 2026)
- `cursor_style() -> CursorStyle`: Get cursor style (block, underline, bar)
- `cursor_color() -> tuple[int, int, int] | None`: Get cursor color (RGB)
- `default_fg() -> tuple[int, int, int] | None`: Get default foreground color
- `default_bg() -> tuple[int, int, int] | None`: Get default background color
- `set_cursor_style(style: CursorStyle)`: Set cursor style
- `set_cursor_color(r: int, g: int, b: int)`: Set cursor color (RGB)
- `set_default_fg(r: int, g: int, b: int)`: Set default foreground color
- `set_default_bg(r: int, g: int, b: int)`: Set default background color
- `query_cursor_color()`: Query cursor color (response in drain_responses())
- `query_default_fg()`: Query default foreground color (response in drain_responses())
- `query_default_bg()`: Query default background color (response in drain_responses())
- `current_directory() -> str | None`: Get current working directory (OSC 7)
- `accept_osc7() -> bool`: Check if OSC 7 (CWD) is accepted
- `set_accept_osc7(accept: bool)`: Set whether to accept OSC 7 sequences
- `disable_insecure_sequences() -> bool`: Check if insecure sequences are disabled
- `set_disable_insecure_sequences(disable: bool)`: Disable insecure/dangerous sequences
- `shell_integration_state() -> ShellIntegration`: Get shell integration state (OSC 133)
- `get_paste_start() -> tuple[int, int] | None`: Get bracketed paste start position
- `get_paste_end() -> tuple[int, int] | None`: Get bracketed paste end position
- `paste(text: str)`: Simulate bracketed paste
- `get_focus_in_event() -> str`: Get focus-in event sequence
- `get_focus_out_event() -> str`: Get focus-out event sequence
- `drain_responses() -> list[str]`: Drain all pending terminal responses (DA, DSR, etc.)
- `drain_notifications() -> list[tuple[str, str]]`: Drain OSC 9/777 notifications (title, message)
- `take_notifications() -> list[tuple[str, str]]`: Take notifications without removing
- `has_pending_responses() -> bool`: Check if responses are pending
- `has_notifications() -> bool`: Check if notifications are pending
- `resize_pixels(width_px: int, height_px: int)`: Resize terminal by pixel dimensions
- `graphics_count() -> int`: Get count of Sixel graphics stored
- `graphics_at_row(row: int) -> list[Graphic]`: Get Sixel graphics at specific row
- `clear_graphics()`: Clear all Sixel graphics
- `create_snapshot() -> ScreenSnapshot`: Create snapshot of current screen state
- `flush_synchronized_updates()`: Flush synchronized updates buffer (DEC 2026)
- `simulate_mouse_event(...)`: Simulate mouse event for testing
- `export_text() -> str`: Export entire buffer (scrollback + current screen) as plain text without styling
- `export_styled() -> str`: Export entire buffer (scrollback + current screen) with ANSI styling
- `export_html(include_styles: bool = True) -> str`: Export current screen as HTML with full styling (full document or content only)
- `screenshot(format, font_path, font_size, include_scrollback, padding, quality, render_cursor, cursor_color, sixel_mode, scrollback_offset, link_color, bold_color, use_bold_color) -> bytes`: Take screenshot and return image bytes
- `screenshot_to_file(path, format, font_path, font_size, include_scrollback, padding, quality, render_cursor, cursor_color, sixel_mode, scrollback_offset, link_color, bold_color, use_bold_color)`: Take screenshot and save to file

#### Text Extraction Utilities

- `get_word_at(col: int, row: int, word_chars: str | None = None) -> str | None`: Extract word at cursor position
- `get_url_at(col: int, row: int) -> str | None`: Detect and extract URL at cursor position
- `get_line_unwrapped(row: int) -> str | None`: Get full logical line following wrapping
- `select_word(col: int, row: int, word_chars: str | None = None) -> tuple[tuple[int, int], tuple[int, int]] | None`: Get word boundaries for smart selection
- `find_matching_bracket(col: int, row: int) -> tuple[int, int] | None`: Find matching bracket/parenthesis. Supports (), [], {}, <>. Returns position of matching bracket or None
- `select_semantic_region(col: int, row: int, delimiters: str) -> str | None`: Extract content between delimiters. Supports (), [], {}, <>, "", '', ``. Returns content or None

#### Content Search

- `find_text(pattern: str, case_sensitive: bool = True) -> list[tuple[int, int]]`: Find all occurrences of text in visible screen
- `find_next(pattern: str, from_col: int, from_row: int, case_sensitive: bool = True) -> tuple[int, int] | None`: Find next occurrence from position

#### Buffer Statistics

- `get_stats() -> dict[str, int]`: Get terminal statistics (cols, rows, scrollback_lines, total_cells, non_whitespace_lines, graphics_count, estimated_memory_bytes)
- `count_non_whitespace_lines() -> int`: Count lines containing non-whitespace characters
- `get_scrollback_usage() -> tuple[int, int]`: Get scrollback usage (used_lines, max_capacity)

#### Static Utility Methods

These methods can be called on the class itself (e.g., `Terminal.strip_ansi(text)`):

- `Terminal.strip_ansi(text: str) -> str`: Remove all ANSI escape sequences from text
- `Terminal.measure_text_width(text: str) -> int`: Measure display width accounting for wide characters and ANSI codes
- `Terminal.parse_color(color_string: str) -> tuple[int, int, int] | None`: Parse color from hex (#RRGGBB), rgb(r,g,b), or name

### PtyTerminal Class

A terminal emulator with PTY (pseudo-terminal) support for running interactive shell sessions.

#### Constructor

```python
PtyTerminal(cols: int, rows: int, scrollback: int = 10000)
```

Create a new PTY-enabled terminal with specified dimensions.

- `cols`: Number of columns (width)
- `rows`: Number of rows (height)
- `scrollback`: Maximum number of scrollback lines (default: 10000)

#### PTY-Specific Methods

- `spawn(cmd: str, args: list[str] = [], env: dict[str, str] | None = None, cwd: str | None = None)`: Spawn a command with arguments
- `spawn_shell(shell: str | None = None)`: Spawn a shell (defaults to /bin/bash)
- `write(data: bytes)`: Write bytes to the PTY
- `write_str(text: str)`: Write string to the PTY (convenience method)
- `is_running() -> bool`: Check if the child process is still running
- `wait() -> int | None`: Wait for child process to exit and return exit code
- `try_wait() -> int | None`: Non-blocking check if child has exited
- `kill()`: Forcefully terminate the child process
- `update_generation() -> int`: Get current update generation counter
- `has_updates_since(generation: int) -> bool`: Check if terminal updated since generation
- `send_resize_pulse()`: Send SIGWINCH to child process after resize
- `get_default_shell() -> str`: Get the default shell path

**Note**: PtyTerminal inherits all methods from Terminal class listed above.

#### Context Manager Support

```python
with PtyTerminal(80, 24) as term:
    term.spawn_shell()
    term.write_str("echo 'Hello'\n")
    # Automatic cleanup on exit
```

### Attributes Class

Represents text attributes for a cell.

#### Properties

- `bold: bool`: Bold text
- `dim: bool`: Dim text
- `italic: bool`: Italic text
- `underline: bool`: Underlined text
- `blink: bool`: Blinking text
- `reverse: bool`: Reverse video
- `hidden: bool`: Hidden text
- `strikethrough: bool`: Strikethrough text

### CursorStyle Enum

Cursor display styles (DECSCUSR):
- `CursorStyle.BlinkingBlock`: Blinking block cursor (default)
- `CursorStyle.SteadyBlock`: Steady block cursor
- `CursorStyle.BlinkingUnderline`: Blinking underline cursor
- `CursorStyle.SteadyUnderline`: Steady underline cursor
- `CursorStyle.BlinkingBar`: Blinking bar/I-beam cursor
- `CursorStyle.SteadyBar`: Steady bar/I-beam cursor

### UnderlineStyle Enum

Text underline styles:
- `UnderlineStyle.None_`: No underline
- `UnderlineStyle.Straight`: Straight underline (default)
- `UnderlineStyle.Double`: Double underline
- `UnderlineStyle.Curly`: Curly underline (for spell check)
- `UnderlineStyle.Dotted`: Dotted underline
- `UnderlineStyle.Dashed`: Dashed underline

### ShellIntegration Class

Shell integration state (OSC 133 and OSC 7):
- `in_prompt: bool`: True if currently in prompt (marker A)
- `in_command_input: bool`: True if currently in command input (marker B)
- `in_command_output: bool`: True if currently in command output (marker C)
- `current_command: str | None`: The command that was executed
- `last_exit_code: int | None`: Exit code from last command (marker D)
- `cwd: str | None`: Current working directory from OSC 7

### Graphic Class

Sixel graphic metadata:
- `row: int`: Display row
- `col: int`: Display column
- `width: int`: Width in pixels
- `height: int`: Height in pixels
- `data: bytes`: Image data

### ScreenSnapshot Class

Immutable snapshot of screen state:
- `content() -> str`: Get full screen content
- `cursor_position() -> tuple[int, int]`: Cursor position at snapshot time
- `size() -> tuple[int, int]`: Terminal dimensions

## Supported ANSI/VT Sequences

This terminal emulator provides comprehensive VT100/VT220/VT320/VT420 compatibility, matching iTerm2's feature set.

### Cursor Movement (VT100)

- `ESC[<n>A`: Cursor up n lines (CUU)
- `ESC[<n>B`: Cursor down n lines (CUD)
- `ESC[<n>C`: Cursor forward n columns (CUF)
- `ESC[<n>D`: Cursor back n columns (CUB)
- `ESC[<n>E`: Cursor next line (CNL)
- `ESC[<n>F`: Cursor previous line (CPL)
- `ESC[<n>G`: Cursor horizontal absolute (CHA)
- `ESC[<row>;<col>H`: Cursor position (CUP)
- `ESC[<row>;<col>f`: Cursor position (HVP - alternative)
- `ESC[<n>d`: Line position absolute (VPA)
- `ESC[s`: Save cursor position (ANSI.SYS)
- `ESC[u`: Restore cursor position (ANSI.SYS)
- `ESC 7`: Save cursor (DECSC)
- `ESC 8`: Restore cursor (DECRC)

### Display Control (VT100)

- `ESC[<n>J`: Erase in display (ED)
  - `n=0`: Clear from cursor to end
  - `n=1`: Clear from beginning to cursor
  - `n=2`: Clear entire screen
  - `n=3`: Clear entire screen and scrollback
- `ESC[<n>K`: Erase in line (EL)
  - `n=0`: Clear from cursor to end of line
  - `n=1`: Clear from beginning of line to cursor
  - `n=2`: Clear entire line

### Line/Character Editing (VT220)

- `ESC[<n>L`: Insert n blank lines (IL)
- `ESC[<n>M`: Delete n lines (DL)
- `ESC[<n>@`: Insert n blank characters (ICH)
- `ESC[<n>P`: Delete n characters (DCH)
- `ESC[<n>X`: Erase n characters (ECH)

### Rectangle Operations (VT420)

Advanced text editing operations that work on rectangular regions of the screen:

- `ESC[<Pc>;<Pt>;<Pl>;<Pb>;<Pr>$x`: Fill Rectangular Area (DECFRA)
  - `Pc`: Character code to fill (e.g., 88 for 'X', 42 for '*')
  - `Pt`: Top row (1-indexed)
  - `Pl`: Left column (1-indexed)
  - `Pb`: Bottom row (1-indexed)
  - `Pr`: Right column (1-indexed)
  - Fills rectangle with specified character using current text attributes

- `ESC[<Pts>;<Pls>;<Pbs>;<Prs>;<Pps>;<Ptd>;<Pld>;<Ppd>$v`: Copy Rectangular Area (DECCRA)
  - `Pts`, `Pls`, `Pbs`, `Prs`: Source rectangle (top, left, bottom, right)
  - `Pps`: Source page (use 1 for current screen)
  - `Ptd`, `Pld`: Destination position (top, left)
  - `Ppd`: Destination page (use 1 for current screen)
  - Copies rectangular region to new location

- `ESC[<Pt>;<Pl>;<Pb>;<Pr>${`: Selective Erase Rectangular Area (DECSERA)
  - `Pt`: Top row (1-indexed)
  - `Pl`: Left column (1-indexed)
  - `Pb`: Bottom row (1-indexed)
  - `Pr`: Right column (1-indexed)
  - Selectively erases rectangle (respects character protection attribute)

- `ESC[<Pt>;<Pl>;<Pb>;<Pr>$z`: Erase Rectangular Area (DECERA)
  - `Pt`: Top row (1-indexed)
  - `Pl`: Left column (1-indexed)
  - `Pb`: Bottom row (1-indexed)
  - `Pr`: Right column (1-indexed)
  - Unconditionally erases rectangle (ignores protection)

- `ESC[<Pt>;<Pl>;<Pb>;<Pr>;<Ps>$r`: Change Attributes in Rectangular Area (DECCARA)
  - `Pt`, `Pl`, `Pb`, `Pr`: Rectangle coordinates (top, left, bottom, right)
  - `Ps`: SGR attributes to apply (0=reset, 1=bold, 4=underline, 5=blink, 7=reverse, 8=hidden)
  - Changes text attributes in rectangle

- `ESC[<Pt>;<Pl>;<Pb>;<Pr>;<Ps>$t`: Reverse Attributes in Rectangular Area (DECRARA)
  - `Pt`, `Pl`, `Pb`, `Pr`: Rectangle coordinates (top, left, bottom, right)
  - `Ps`: Attributes to reverse (0=all, 1=bold, 4=underline, 5=blink, 7=reverse, 8=hidden)
  - Toggles attributes in rectangle

- `ESC[<Pi>;<Pg>;<Pt>;<Pl>;<Pb>;<Pr>*y`: Request Checksum of Rectangular Area (DECRQCRA)
  - `Pi`: Request ID
  - `Pg`: Page number (use 1 for current screen)
  - `Pt`, `Pl`, `Pb`, `Pr`: Rectangle coordinates
  - Response: `DCS Pi ! ~ xxxx ST` (16-bit checksum in hex)

- `ESC[<Ps>*x`: Select Attribute Change Extent (DECSACE)
  - `Ps = 0` or `1`: Stream mode (attributes wrap at line boundaries)
  - `Ps = 2`: Rectangle mode (strict rectangular boundaries, default)
  - Affects how DECCARA and DECRARA apply attributes

**Use Cases**: Efficient text manipulation in editors (vim, emacs), drawing box characters, clearing specific screen regions without affecting surrounding content, attribute modification without changing text, verification of screen regions via checksums.

### Scrolling (VT100/VT220)

- `ESC[<n>S`: Scroll up n lines (SU)
- `ESC[<n>T`: Scroll down n lines (SD)
- `ESC[<top>;<bottom>r`: Set scrolling region (DECSTBM)
- `ESC M`: Reverse index (RI) - scroll down at top
- `ESC D`: Index (IND) - scroll up at bottom
- `ESC E`: Next line (NEL)

### Colors and Attributes (VT100/ECMA-48)

- `ESC[0m`: Reset all attributes (SGR 0)
- `ESC[1m`: Bold
- `ESC[2m`: Dim
- `ESC[3m`: Italic
- `ESC[4m`: Underline (basic, defaults to straight)
- `ESC[4:0m`: No underline (explicit)
- `ESC[4:1m`: Straight underline (default)
- `ESC[4:2m`: Double underline
- `ESC[4:3m`: Curly underline (spell check, errors)
- `ESC[4:4m`: Dotted underline
- `ESC[4:5m`: Dashed underline
- `ESC[5m`: Blink
- `ESC[7m`: Reverse
- `ESC[8m`: Hidden
- `ESC[9m`: Strikethrough
- `ESC[22m`: Normal intensity (not bold or dim)
- `ESC[23m`: Not italic
- `ESC[24m`: Not underlined
- `ESC[25m`: Not blinking
- `ESC[27m`: Not reversed
- `ESC[28m`: Not hidden
- `ESC[29m`: Not strikethrough
- `ESC[30-37m`: Foreground colors (basic)
- `ESC[40-47m`: Background colors (basic)
- `ESC[90-97m`: Bright foreground colors (aixterm)
- `ESC[100-107m`: Bright background colors (aixterm)
- `ESC[38;5;<n>m`: 256-color foreground
- `ESC[48;5;<n>m`: 256-color background
- `ESC[38;2;<r>;<g>;<b>m`: RGB/true color foreground
- `ESC[48;2;<r>;<g>;<b>m`: RGB/true color background
- `ESC[39m`: Default foreground color
- `ESC[49m`: Default background color

### Tab Stops (VT100)

- `ESC H`: Set tab stop at current column (HTS)
- `ESC[<n>g`: Tab clear (TBC)
  - `n=0`: Clear tab at current column
  - `n=3`: Clear all tabs
- `ESC[<n>I`: Cursor forward tabulation (CHT)
- `ESC[<n>Z`: Cursor backward tabulation (CBT)

### Terminal Modes (DEC Private Modes)

- `ESC[?1h/l`: Application cursor keys (DECCKM)
- `ESC[?6h/l`: Origin mode (DECOM)
- `ESC[?7h/l`: Auto wrap mode (DECAWM)
- `ESC[?25h/l`: Show/hide cursor (DECTCEM)
- `ESC[?47h/l`: Alternate screen buffer
- `ESC[?1047h/l`: Alternate screen buffer (alternate)
- `ESC[?1048h/l`: Save/restore cursor
- `ESC[?1049h/l`: Save cursor and use alternate screen

### Mouse Support (xterm)

- `ESC[?1000h/l`: Normal mouse tracking
- `ESC[?1002h/l`: Button event mouse tracking
- `ESC[?1003h/l`: Any event mouse tracking
- `ESC[?1005h/l`: UTF-8 mouse encoding
- `ESC[?1006h/l`: SGR mouse encoding
- `ESC[?1015h/l`: URXVT mouse encoding

### Advanced Features

- `ESC[?1004h/l`: Focus tracking
- `ESC[?2004h/l`: Bracketed paste mode
- `ESC[?2026h/l`: Synchronized updates (DEC 2026) - Batch screen updates for flicker-free rendering

### Kitty Keyboard Protocol

Progressive enhancement for keyboard handling with flags for disambiguation and event reporting:

- `CSI = flags ; mode u`: Set keyboard protocol mode
  - `flags`: Bitmask (1=disambiguate, 2=report events, 4=alternate keys, 8=report all, 16=associated text)
  - `mode`: 0=disable, 1=set, 2=lock, 3=report
- `CSI ? u`: Query current keyboard flags (response: `CSI ? flags u`)
- `CSI > flags u`: Push current flags to stack and set new flags
- `CSI < count u`: Pop flags from stack (count times)

**Note**: Flags are maintained separately for main and alternate screen buffers with independent stacks.

### Device Queries (VT100/VT220)

- `ESC[<n>n`: Device Status Report (DSR)
- `ESC[c`: Device Attributes (DA)

### OSC Sequences

- `OSC 0;<title>ST`: Set window title (icon + title)
- `OSC 2;<title>ST`: Set window title
- `OSC 7;<cwd>ST`: Set current working directory
- `OSC 8;;<url>ST`: Hyperlinks (iTerm2/VTE compatible) - **Full support with clickable TUI rendering**
- `OSC 52;c;<data>ST`: Clipboard operations (xterm/iTerm2 compatible) - **Works over SSH without X11!**
  - `<data>`: base64 encoded text to copy to clipboard
  - `?`: Query clipboard (requires `set_allow_clipboard_read(true)` for security)
  - Empty data clears clipboard
- `OSC 133;<marker>ST`: Shell integration (iTerm2/VSCode)
  - `A`: Prompt start
  - `B`: Command start
  - `C`: Command executed
  - `D;<exit_code>`: Command finished
- `OSC 9;<message>ST`: Notifications (iTerm2/ConEmu style) - Send desktop-style notifications
  - Simple format with message only (no title)
- `OSC 777;notify;<title>;<message>ST`: Notifications (urxvt style) - Structured notifications
  - Supports both title and message
  - Use for desktop notifications, alerts, or completion notices

### Control Characters

- `BEL` (0x07): Bell
- `BS` (0x08): Backspace
- `HT` (0x09): Horizontal tab
- `LF` (0x0A): Line feed
- `CR` (0x0D): Carriage return

### Reset Sequences

- `ESC c`: Reset to initial state (RIS)

## Examples

See the `examples/` directory for complete usage examples:

### Terminal Emulation Examples

#### Basic Examples
- `basic_usage_improved.py`: Enhanced basic usage with visual output
- `colors_demo.py`: Color support demonstration
- `cursor_movement.py`: Cursor control examples
- `scrollback_demo.py`: Scrollback buffer usage
- `text_attributes.py`: Text styling examples
- `unicode_emoji.py`: Unicode and emoji support demonstration

#### Advanced Features
- `alt_screen.py`: Alternate screen buffer
- `mouse_tracking.py`: Mouse event handling
- `bracketed_paste.py`: Bracketed paste mode demonstration
- `synchronized_updates.py`: Synchronized updates (DEC 2026) for flicker-free rendering
- `shell_integration.py`: Shell integration (OSC 133)
- `test_osc52_clipboard.py`: OSC 52 clipboard operations (SSH clipboard support)
- `test_kitty_keyboard.py`: Kitty Keyboard Protocol demonstration
- `test_underline_styles.py`: Underline styles (SGR 4:x) for modern text decoration
- `notifications.py`: OSC 9 / OSC 777 notification support (desktop-style alerts)
- `rectangle_operations.py`: VT420 rectangle operations (DECFRA, DECCRA, DECSERA, DECERA, DECCARA, DECRARA, DECRQCRA, DECSACE)
- `hyperlink_demo.py`: OSC 8 hyperlinks (clickable URLs in terminal)

#### Graphics and Visual
- `display_image_sixel.py`: Display images using Sixel graphics
- `test_sixel_simple.py`: Simple Sixel graphics demonstration
- `test_sixel_display.py`: Advanced Sixel graphics testing
- `screenshot_demo.py`: Comprehensive screenshot feature demonstration

#### TUI Applications
- `test_tui_clipboard.py`: TUI clipboard integration demonstration
- `feature_showcase.py`: Comprehensive feature showcase with TUI

### PTY Examples
- `pty_basic.py`: Basic PTY usage - spawn commands and capture output
- `pty_shell.py`: Interactive shell sessions
- `pty_custom_env.py`: Custom environment variables and working directory
- `pty_resize.py`: Dynamic terminal resizing with SIGWINCH
- `pty_with_par_term.py`: Using custom par-term terminfo for optimal compatibility
- `pty_multiple.py`: Managing multiple concurrent PTY sessions
- `pty_event_loop.py`: Event loop integration with update tracking
- `pty_mouse_events.py`: Mouse event handling in PTY sessions

## TUI Demo Application

> **Note**: A full-featured TUI (Text User Interface) application for this terminal emulator is available in the sister project [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust). The TUI provides an interactive terminal application with themes, clipboard integration, and more.

**Installation**: `pip install par-term-emu-tui-rust`

## Security

**Important**: When using PTY functionality, follow security best practices to prevent command injection and other vulnerabilities.

See [SECURITY.md](docs/SECURITY.md) for detailed security guidelines including:
- Command injection prevention
- Environment variable security
- Input validation
- Resource limits
- Privilege management

## Running Tests

```bash
# Run Rust tests
cargo test

# Run Python tests (requires pytest)
uv pip install pytest
pytest tests/
```

## Performance

The library is implemented in Rust for high performance:

- Zero-copy operations where possible
- Efficient grid representation
- Fast ANSI sequence parsing using the `vte` crate
- Minimal Python/Rust boundary crossings

## Architecture

The library consists of several key components:

### Terminal Emulation Core
- **Cell**: Represents a single terminal cell with character, colors, and attributes
- **Grid**: Manages the 2D terminal buffer and scrollback
- **Cursor**: Tracks cursor position and visibility
- **Color**: Handles various color formats (named, 256-color, RGB)
- **Terminal**: Main terminal emulator that ties everything together

### PTY Support
- **PtySession**: Manages PTY and child process lifecycle
- **PtyError**: Error types for PTY operations
- **Reader Thread**: Background thread that processes PTY output
- **Update Generation**: Efficient change detection for event loops

### Python Bindings
- **PyTerminal**: Python wrapper for Terminal (ANSI parsing)
- **PyPtyTerminal**: Python wrapper for PtySession (interactive shells)
- **PyO3**: Zero-cost bindings between Rust and Python

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

### Development Setup

1. **Clone the repository and set up the environment:**
   ```bash
   git clone https://github.com/paulrobello/par-term-emu-core-rust.git
   cd par-term-emu-core-rust
   make setup-venv  # Create virtual environment and install dependencies
   ```

2. **Install pre-commit hooks (recommended):**
   ```bash
   make pre-commit-install
   ```
   This will automatically run all quality checks (formatting, linting, type checking, tests) before each commit.

3. **Build and test:**
   ```bash
   make dev          # Build the library in development mode
   make test-python  # Run tests
   ```

### Code Quality

All contributions must pass the following checks:
- **Rust formatting**: `cargo fmt`
- **Rust linting**: `cargo clippy -- -D warnings`
- **Python formatting**: `make fmt-python`
- **Python linting**: `make lint-python`
- **Python type checking**: `pyright`
- **Python tests**: `make test-python`

**TIP**: Use `make pre-commit-install` to automate all these checks on every commit!

For more detailed development instructions, see [CLAUDE.md](CLAUDE.md).

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

Paul Robello - probello@gmail.com
