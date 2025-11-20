# Par Term Emu Core Rust

[![PyPI](https://img.shields.io/pypi/v/par_term_emu_core_rust)](https://pypi.org/project/par_term_emu_core_rust/)
[![PyPI - Python Version](https://img.shields.io/pypi/pyversions/par_term_emu_core_rust.svg)](https://pypi.org/project/par_term_emu_core_rust/)
![Runs on Linux | MacOS | Windows](https://img.shields.io/badge/runs%20on-Linux%20%7C%20MacOS%20%7C%20Windows-blue)
![Arch x86-64 | ARM | AppleSilicon](https://img.shields.io/badge/arch-x86--64%20%7C%20ARM%20%7C%20AppleSilicon-blue)
![PyPI - Downloads](https://img.shields.io/pypi/dm/par_term_emu_core_rust)
![PyPI - License](https://img.shields.io/pypi/l/par_term_emu_core_rust)

A comprehensive terminal emulator library written in Rust with Python bindings for Python 3.12+. Provides VT100/VT220/VT320/VT420/VT520 compatibility with PTY support, matching iTerm2's feature set.

[!["Buy Me A Coffee"](https://www.buymeacoffee.com/assets/img/custom_images/orange_img.png)](https://buymeacoffee.com/probello3)

## What's New in 0.8.0

- **Keyboard Protocol Reset Fix**: Automatically reset Kitty Keyboard Protocol flags when exiting alternate screen buffer
  - Prevents TUI apps from leaving keyboard in bad state if they fail to disable protocol on exit
  - Ensures clean terminal state after TUI app termination

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
- **Unicode Support** - Full Unicode including emoji and wide characters

### Modern Features

- **Alternate Screen Buffer** - Full support with automatic cleanup
- **Mouse Support** - Multiple tracking modes and encodings (X10, Normal, Button, Any, SGR, URXVT)
- **Bracketed Paste Mode** - Safe paste handling
- **Focus Tracking** - Focus in/out events
- **OSC 8 Hyperlinks** - Clickable URLs in terminal (full TUI support)
- **OSC 52 Clipboard** - Copy/paste over SSH without X11
- **OSC 9/777 Notifications** - Desktop-style alerts and notifications
- **Shell Integration** - OSC 133 (iTerm2/VSCode compatible)
- **Sixel Graphics** - Inline graphics with half-block rendering
- **Kitty Keyboard Protocol** - Progressive keyboard enhancement with auto-reset on alternate screen exit
- **Synchronized Updates (DEC 2026)** - Flicker-free rendering
- **Tmux Control Protocol** - Control mode integration support

### PTY Support

- **Interactive Shell Sessions** - Spawn and control shell processes
- **Bidirectional I/O** - Send input and receive output
- **Process Management** - Start, stop, and monitor child processes
- **Dynamic Resizing** - Resize with SIGWINCH signal
- **Environment Control** - Custom environment variables and working directory
- **Event Loop Integration** - Non-blocking update detection
- **Cross-Platform** - Linux, macOS, and Windows via portable-pty

### Screenshots and Export

- **Multiple Formats** - PNG, JPEG, BMP, SVG (vector), HTML
- **Embedded Font** - JetBrains Mono bundled - no installation required
- **Programming Ligatures** - =>, !=, >=, and other code ligatures
- **True Font Rendering** - High-quality antialiasing for raster formats
- **Color Emoji Support** - Full emoji rendering with automatic font fallback
- **Session Recording** - Record/replay sessions (asciicast v2, JSON)
- **Export Functions** - Plain text, ANSI styled, HTML export

### Utility Functions

- **Text Extraction** - Smart word/URL detection, selection boundaries, bracket matching
- **Content Search** - Find text with case-sensitive/insensitive matching
- **Buffer Statistics** - Memory usage, cell counts, graphics count
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
- **[VT Feature Parity](docs/VT_FEATURE_PARITY.md)** - iTerm2 compatibility details
- **[Fonts](docs/FONTS.md)** - Font configuration and rendering

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

## Examples

See the `examples/` directory for comprehensive examples:

### Basic Examples
- `basic_usage_improved.py` - Enhanced basic usage
- `colors_demo.py` - Color support
- `cursor_movement.py` - Cursor control
- `text_attributes.py` - Text styling
- `unicode_emoji.py` - Unicode/emoji support

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
- `screenshot_demo.py` - Screenshot features
- `feature_showcase.py` - Comprehensive TUI showcase

### PTY Examples
- `pty_basic.py` - Basic PTY usage
- `pty_shell.py` - Interactive shells
- `pty_resize.py` - Dynamic resizing
- `pty_event_loop.py` - Event loop integration
- `pty_mouse_events.py` - Mouse in PTY

## TUI Demo Application

A full-featured TUI (Text User Interface) application is available in the sister project [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust).

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
- **GitHub:** [https://github.com/paulrobello/par-term-emu-core-rust](https://github.com/paulrobello/par-term-emu-core-rust)
- **TUI Application:** [https://github.com/paulrobello/par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust)
- **Documentation:** See [docs/](docs/) directory
- **Examples:** See [examples/](examples/) directory
