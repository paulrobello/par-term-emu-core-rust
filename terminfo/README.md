# PAR Terminal Emulator - Terminfo Definition

This directory contains the terminfo definition for par-term-emu, which describes the terminal's capabilities to applications.

## Overview

The `par-term` terminfo entry provides:

- **VT220 Base Compatibility**: Full VT220 feature set
- **256 Color Support**: Compatible with xterm-256color
- **24-bit True Color**: RGB color support (16 million colors)
- **Sixel Graphics**: Inline image display support
- **Modern Terminal Features**:
  - Bracketed paste mode
  - Focus tracking
  - Mouse tracking (multiple modes and encodings)
  - Synchronized updates (DEC 2026)
  - OSC 52 clipboard integration
  - OSC 8 hyperlinks
  - Shell integration (OSC 133)
  - Kitty Keyboard Protocol support
- **VT420 Rectangle Operations**: Advanced text editing
- **Underline Styles**: Multiple underline variants (straight, double, curly, dotted, dashed)

## Installation

### System-wide Installation

To install the terminfo definition system-wide (requires sudo):

```bash
sudo tic -x terminfo/par-term.ti
```

This installs to `/usr/share/terminfo/` (Linux) or `/usr/share/misc/terminfo/` (macOS).

### User Installation

To install for the current user only:

```bash
tic -x -o ~/.terminfo terminfo/par-term.ti
```

### Using the Install Script

A convenience script is provided:

```bash
# Install for current user
./terminfo/install.sh

# Install system-wide
sudo ./terminfo/install.sh --system
```

## Usage

After installation, set the `TERM` environment variable:

```bash
export TERM=par-term
```

Or when spawning a PTY session:

```python
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
term.spawn(
    "/bin/bash",
    env={"TERM": "par-term", "COLORTERM": "truecolor"}
)
```

## Verification

After installation, verify the terminfo is available:

```bash
# Check if installed
infocmp par-term

# Test color support
tput colors

# Test true color
tput setrgbf 255 128 0
echo "Orange text"
tput sgr0
```

## Compatibility

The `par-term` terminfo is designed to be compatible with:

- **xterm-256color**: Full backward compatibility for 256-color applications
- **VT220**: Classic terminal applications
- **Modern terminals**: Support for features like true color, bracketed paste, etc.

### Fallback

If `par-term` is not available on a system, the terminal defaults to `xterm-256color`, which provides good compatibility with most applications.

## Terminal Capabilities

<details>
<summary>Click to expand capability list</summary>

### Basic Capabilities
- `am`: Automatic right margin
- `bce`: Background color erase
- `km`: Has meta key
- `mir`: Safe to move in insert mode
- `xenl`: Newline ignored after 80 cols

### Color Support
- `colors#256`: 256 color palette
- `pairs#65536`: 65536 color pairs
- `setaf`/`setab`: 256-color foreground/background
- `setrgbf`/`setrgbb`: 24-bit RGB true color

### Text Attributes
- `bold`, `dim`, `italic`: Text styling
- `smul`/`rmul`: Underline on/off
- `smulx`: Underline styles (straight, double, curly, dotted, dashed)
- `blink`, `rev`, `invis`: Additional attributes
- `smxx`/`rmxx`: Strikethrough

### Cursor Control
- `cup`: Cursor position
- `cuu`/`cud`/`cuf`/`cub`: Cursor movement
- `home`, `hpa`, `vpa`: Positioning
- `civis`/`cnorm`/`cvvis`: Cursor visibility

### Editing
- `il`/`dl`: Insert/delete lines
- `ich`/`dch`: Insert/delete characters
- `ech`: Erase characters
- `el`/`ed`: Erase line/display

### Scrolling
- `csr`: Change scroll region
- `ind`/`ri`: Index/reverse index
- `indn`/`rin`: Scroll up/down

### Special Features
- `kmous`: Mouse event sequences
- `XM`: Mouse mode control
- `BD`/`BE`: Bracketed paste
- `Sync`: Synchronized updates
- `Ss`/`Se`: Sixel graphics
- `Hl`: Hyperlinks (OSC 8)
- `Ms`: Clipboard (OSC 52)

</details>

## Building from Source

The terminfo source file (`par-term.ti`) is human-readable and can be edited if needed. After making changes, recompile:

```bash
tic -x terminfo/par-term.ti
```

## Troubleshooting

### Command not found: tic

Install ncurses utilities:

```bash
# Ubuntu/Debian
sudo apt-get install ncurses-bin

# macOS (should be pre-installed)
# If missing, install via Homebrew
brew install ncurses

# Fedora/RHEL
sudo dnf install ncurses
```

### Terminal capabilities not recognized

Some older systems may not support extended terminfo capabilities. In this case, use the fallback:

```bash
export TERM=xterm-256color
```

### Colors not working

Ensure both `TERM` and `COLORTERM` are set:

```bash
export TERM=par-term
export COLORTERM=truecolor
```

## References

- [terminfo(5) man page](https://man7.org/linux/man-pages/man5/terminfo.5.html)
- [ncurses terminfo](https://invisible-island.net/ncurses/man/terminfo.5.html)
- [xterm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)
- [VT100 User Guide](https://vt100.net/docs/vt100-ug/)
