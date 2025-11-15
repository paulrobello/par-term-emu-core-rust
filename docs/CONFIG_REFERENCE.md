# Terminal Core Configuration Reference

Comprehensive reference for par-term-emu-core-rust terminal emulator configuration options, modes, and settings.

This document covers the **terminal core configuration** (Rust library internals) for the par-term-emu terminal emulator. For TUI application settings, see the [par-term-emu-rust project](https://github.com/paulrobello/par-term-emu-rust).

## Table of Contents

- [Overview](#overview)
- [Terminal Construction](#terminal-construction)
- [Runtime Configuration](#runtime-configuration)
- [Terminal Modes](#terminal-modes)
- [Core Mouse Configuration](#core-mouse-configuration)
- [Core Security Settings](#core-security-settings)
- [Keyboard Protocol](#keyboard-protocol)
- [Color Configuration](#color-configuration)
- [Configuration Validation](#configuration-validation)
- [Configuration Best Practices](#configuration-best-practices)
- [Common Configuration Patterns](#common-configuration-patterns)
- [Environment Variables](#environment-variables)
- [Related Documentation](#related-documentation)

---

## Overview

The par-term-emu-core-rust library provides low-level terminal emulation with VT100/VT220/VT320/VT420 compatibility. Configuration is done programmatically via the Python or Rust API, or dynamically via escape sequences.

**Configuration layers:**
- **Terminal Core** (this document): Low-level terminal emulator settings (dimensions, modes, protocols)
- **TUI Layer**: Application-level settings for the TUI (see [par-term-emu-rust](https://github.com/paulrobello/par-term-emu-rust))

**Key Distinction**: Core settings control the *terminal emulation behavior* (VT modes, color parsing, escape sequences), while TUI settings control the *application experience* (selection, clipboard, themes).

---

## Terminal Construction

These parameters must be provided when creating a new Terminal instance.

### Required Parameters

| Parameter | Type | Constraints | Description |
|-----------|------|-------------|-------------|
| `cols` | `usize` | > 0 | Number of columns (terminal width) |
| `rows` | `usize` | > 0 | Number of rows (terminal height) |

### Optional Parameters

| Parameter | Type | Default | Constraints | Description |
|-----------|------|---------|-------------|-------------|
| `scrollback` | `usize` | 10000 | ≥ 0 | Maximum scrollback buffer size (0 = disabled) |

**Python Example:**
```python
from par_term_emu import Terminal

# Create 80x24 terminal with default scrollback
term = Terminal(cols=80, rows=24)

# Create with custom scrollback
term = Terminal(cols=120, rows=40, scrollback=5000)

# Disable scrollback
term = Terminal(cols=80, rows=24, scrollback=0)
```

**Location:** `Terminal` struct in `src/terminal/mod.rs` and Python bindings in `src/python_bindings/terminal.rs`

---

## Runtime Configuration

These settings can be queried or modified after terminal creation.

### Display Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `pixel_width` | `usize` | 0 | Pixel width for XTWINOPS 14 reporting |
| `pixel_height` | `usize` | 0 | Pixel height for XTWINOPS 14 reporting |
| `title` | `String` | `""` | Window/icon title (set via OSC 0/2) |

**Usage:**
- These are typically set automatically by the terminal emulator host
- `title` is updated via OSC 0/2 escape sequences
- Pixel dimensions are used for XTWINOPS size reporting

### Scroll Region Configuration

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `scroll_region_top` | `usize` | 0 | Top of scrolling region (0-indexed) |
| `scroll_region_bottom` | `usize` | `rows - 1` | Bottom of scrolling region (0-indexed) |
| `left_margin` | `usize` | 0 | Left column margin (DECLRMM) |
| `right_margin` | `usize` | `cols - 1` | Right column margin (DECLRMM) |

**Notes:**
- Set via `CSI r` (DECSTBM) for top/bottom margins
- Set via `CSI s` (DECSLRM) for left/right margins (requires DECLRMM mode)
- Affects scrolling behavior and cursor movement in origin mode

---

## Terminal Modes

These modes control terminal behavior and are typically set via escape sequences.

### Display Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Auto Wrap | `CSI ? 7 h/l` (DECAWM) | `true` | Automatic line wrapping at right margin |
| Origin Mode | `CSI ? 6 h/l` (DECOM) | `false` | Cursor addressing relative to scroll region |
| Reverse Video | `CSI ? 5 h/l` (DECSCNM) | `false` | Globally invert foreground/background colors |

### Cursor Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Application Cursor Keys | `CSI ? 1 h/l` (DECCKM) | `false` | Application vs normal cursor key mode |
| Cursor Visibility | `CSI ? 25 h/l` (DECTCEM) | `true` | Show/hide cursor |

### Editing Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Insert Mode | `CSI 4 h/l` (IRM) | `false` | Insert vs replace mode for character input |
| Line Feed Mode | `CSI 20 h/l` (LNM) | `false` | LF does CR+LF (true) vs LF only (false) |
| Character Protection | `CSI 0/1 " q` (DECSCA) | `false` | Mark characters as protected from erasure |

### Screen Modes

| Mode | VT Sequence | Default | Description | Notes |
|------|-------------|---------|-------------|-------|
| Alternate Screen | `CSI ? 47/1047 h/l` | `false` | Switch to/from alternate screen buffer | No scrollback in alt |
| Alternate + Save Cursor | `CSI ? 1049 h/l` | `false` | Alt screen + cursor save/restore | Combined operation |

### Margin Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Left/Right Margin Mode | `CSI ? 69 h/l` (DECLRMM) | `false` | Enable left/right margin support |

**Note:** DECSLRM (`CSI s`) only works when DECLRMM is enabled.

### Update Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Bracketed Paste | `CSI ? 2004 h/l` | `false` | Wrap pasted content in escape sequences |
| Synchronized Updates | `CSI ? 2026 h/l` | `false` | Batch screen updates for flicker-free rendering |

---

## Core Mouse Configuration

### Mouse Tracking Modes

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| X10 Mouse | `CSI ? 9 h/l` | Off | Report button press only |
| Normal Mouse | `CSI ? 1000 h/l` | Off | Report press and release |
| Button Event Mouse | `CSI ? 1002 h/l` | Off | Report press, release, and drag |
| Any Event Mouse | `CSI ? 1003 h/l` | Off | Report all mouse motion |

**Mouse Mode Values:**
- `MouseMode::Off` - No mouse tracking
- `MouseMode::X10` - Press only
- `MouseMode::Normal` - Press + release
- `MouseMode::Button` - Press + release + drag
- `MouseMode::Any` - All motion

**Implementation:** See `MouseMode` enum in `src/mouse.rs` and usage in `Terminal` struct

### Mouse Encoding Modes

| Encoding | VT Sequence | Default | Description |
|----------|-------------|---------|-------------|
| Default (X11) | - | Yes | Classic X11 encoding (< 223 coords) |
| UTF-8 | `CSI ? 1005 h/l` | No | UTF-8 extended coordinates |
| SGR | `CSI ? 1006 h/l` | No | Recommended: `CSI < ... M/m` format |
| URXVT | `CSI ? 1015 h/l` | No | URXVT extended encoding |

**Implementation:** See `MouseEncoding` enum in `src/mouse.rs` and usage in `Terminal` struct

### Focus Tracking

| Mode | VT Sequence | Default | Description |
|------|-------------|---------|-------------|
| Focus Events | `CSI ? 1004 h/l` | `false` | Report focus in/out events |

---

## Core Security Settings

These settings control potentially sensitive or insecure terminal features at the core level.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `allow_clipboard_read` | `bool` | `false` | Allow OSC 52 clipboard queries (security risk) |
| `accept_osc7` | `bool` | `true` | Accept OSC 7 directory tracking |
| `disable_insecure_sequences` | `bool` | `false` | Block OSC 8, 52, 9, 777, and Sixel graphics |

### Security Recommendations

**OSC 52 Clipboard Read:**
- ⚠️ Default: Disabled for security
- Allows applications to read clipboard content
- Only enable in trusted environments
- Write access is always permitted

**Insecure Sequence Blocking:**
- When enabled, blocks:
  - OSC 8 (hyperlinks)
  - OSC 52 (clipboard operations)
  - OSC 9 (iTerm2 notifications)
  - OSC 777 (urxvt notifications)
  - Sixel graphics
- Use in untrusted/sandboxed environments

**OSC 7 Directory Tracking:**
- Generally safe to keep enabled
- Used for shell integration and smart directory tracking
- Disable if you don't need this feature

---

## Keyboard Protocol

### Kitty Keyboard Protocol

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `keyboard_flags` | `u16` | 0 | Kitty keyboard protocol flags |

### Kitty Protocol Flags

| Bit | Value | Description |
|-----|-------|-------------|
| 1 | 0x01 | Disambiguate escape sequences |
| 2 | 0x02 | Report event types (press/release/repeat) |
| 4 | 0x04 | Report alternate keys |
| 8 | 0x08 | Report all keys as escape sequences |
| 16 | 0x10 | Report associated text |

**Set via:**
- `CSI = flags ; mode u` - Set/disable/lock flags
- `CSI ? u` - Query current flags
- `CSI > flags u` - Push flags to stack
- `CSI < count u` - Pop flags from stack

**Implementation:** See keyboard protocol handling in `Terminal` struct CSI dispatch methods.

**Note:** Separate flag stacks are maintained for primary and alternate screens.

---

## Color Configuration

### Default Colors

Default colors can be queried and are used when SGR reset (0) is applied.

| Property | Type | Default | Query Sequence |
|----------|------|---------|----------------|
| `default_fg` | `Color` | White | `OSC 10 ; ? ST` |
| `default_bg` | `Color` | Black | `OSC 11 ; ? ST` |
| `cursor_color` | `Color` | White | `OSC 12 ; ? ST` |

**Query Response Format:**
```
OSC 10;rgb:rrrr/gggg/bbbb ST  (foreground)
OSC 11;rgb:rrrr/gggg/bbbb ST  (background)
OSC 12;rgb:rrrr/gggg/bbbb ST  (cursor)
```

**Implementation:** See OSC color query handling in `Terminal` OSC dispatch methods

### iTerm2 Extended Colors

Additional color configuration options for iTerm2 feature parity:

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `link_color` | `Color` | Blue (#0645ad) | Hyperlink text color (OSC 8) |
| `bold_color` | `Color` | White (#ffffff) | Custom bold text color |
| `cursor_guide_color` | `Color` | Light Blue (#a6e8ff) | Cursor column/row highlight |
| `badge_color` | `Color` | Red (#ff0000) | Badge/notification color |
| `match_color` | `Color` | Yellow (#ffff00) | Search/match highlight color |
| `selection_bg_color` | `Color` | Light Blue (#b5d5ff) | Selection background |
| `selection_fg_color` | `Color` | Black (#000000) | Selection text/foreground |

### iTerm2 Color Behavior Flags

Control whether custom colors are used instead of defaults:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `use_bold_color` | `bool` | `false` | Use custom bold color instead of bright variant |
| `use_underline_color` | `bool` | `false` | Use custom underline color (SGR 58) |
| `use_cursor_guide` | `bool` | `false` | Show cursor guide (column/row highlight) |
| `use_selected_text_color` | `bool` | `false` | Use custom selection text color |
| `smart_cursor_color` | `bool` | `false` | Auto-adjust cursor color based on background |
| `bold_brightening` | `bool` | `true` | Bold ANSI colors 0-7 brighten to 8-15 |

**Notes:**
- These settings provide feature parity with iTerm2's color configuration
- Colors can be queried via OSC sequences (10, 11, 12, etc.)
- Custom colors only apply when corresponding `use_*` flags are enabled
- Bold brightening is a legacy feature for ANSI color compatibility

---

# Integration & Best Practices

## Configuration Validation

### Dimension Constraints

- **Minimum:** 1 column × 1 row
- **Recommended minimum:** 80 columns × 24 rows (VT100 standard)
- **Maximum:** Limited by available memory

### Scrollback Constraints

- **Minimum:** 0 (disabled)
- **Recommended:** 1000-10000 lines
- **Default:** 10000 lines

### Runtime Checks

The terminal validates:
- Column/row indices against current dimensions
- Scroll region bounds (top < bottom, left < right)
- Tab stop positions
- Color values (0-255 for indexed, 0-255 for RGB components)

---

## Configuration Best Practices

### For Application Developers

1. **Start with standard dimensions:** 80×24 or 120×40
2. **Enable bracketed paste** for applications accepting multi-line input
3. **Use synchronized updates** to prevent screen flicker
4. **Enable SGR mouse mode** (1006) for extended coordinate support
5. **Query terminal capabilities** using DA (Device Attributes)

### For Security-Conscious Environments

**TUI Layer:**
1. Set `disable_insecure_sequences = true` in `config.yaml`
2. Set `expose_system_clipboard = false` if clipboard access is sensitive
3. Consider `accept_osc7 = false` for sensitive path information

**Core Layer:**
1. Keep `allow_clipboard_read = false` (default)
2. Monitor OSC 7 usage if working with sensitive paths
3. Consider disabling mouse tracking in production

### For TUI Applications

1. Enable alternate screen (`CSI ? 1047 h`)
2. Use synchronized updates for smooth rendering
3. Save/restore cursor with `CSI ? 1049 h/l`
4. Query terminal size with `CSI 18 t` (XTWINOPS)
5. Set up proper cleanup in signal handlers

### For End Users

**Complete Configuration Example** (`~/.config/par-term-emu/config.yaml`):

```yaml
# ~/.config/par-term-emu/config.yaml

# --- Selection & Clipboard ---
auto_copy_selection: true                 # Auto-copy on select
keep_selection_after_copy: true           # Keep highlight after copy
expose_system_clipboard: true             # OSC 52 access
copy_trailing_newline: true               # Include \n when copying lines
word_characters: "-_.~:/?#[]@!$&'()*+,;=" # Word boundary characters
triple_click_selects_wrapped_lines: true  # Follow wrapping on triple-click

# --- Scrollback ---
scrollback_lines: 10000                   # Scrollback buffer size (0 = unlimited)
max_scrollback_lines: 100000              # Safety limit for unlimited

# --- Cursor ---
cursor_blink_enabled: false               # Enable cursor blinking
cursor_blink_rate: 0.5                    # Blink interval in seconds
cursor_style: "blinking_block"            # Cursor appearance

# --- Paste ---
paste_chunk_size: 0                       # Paste chunking (0 = disabled)
paste_chunk_delay_ms: 10                  # Delay between chunks
paste_warn_size: 100000                   # Warn before large paste

# --- Mouse & Focus ---
focus_follows_mouse: false                # Auto-focus on hover
middle_click_paste: true                  # Paste on middle-click
mouse_wheel_scroll_lines: 3               # Lines per scroll wheel tick

# --- Theme ---
theme: "dark-background"                  # Color theme name

# --- Notifications ---
show_notifications: true                  # Display OSC 9/777 notifications
notification_timeout: 5                   # Notification duration (seconds)

# --- Screenshots ---
screenshot_directory: null                # Auto-detect save directory
screenshot_format: "png"                  # Format: png, jpeg, bmp, svg, html
open_screenshot_after_capture: false      # Auto-open after capture

# --- Shell Behavior ---
exit_on_shell_exit: true                  # Exit TUI when shell exits

# --- Security & Advanced ---
disable_insecure_sequences: false         # Block risky escape sequences
accept_osc7: true                         # Directory tracking (OSC 7)
```

---

## Common Configuration Patterns

### Full-Featured Interactive Application

```python
# Enable all interactive features
term.process(b"\x1b[?1049h")     # Alt screen + save cursor
term.process(b"\x1b[?25h")       # Show cursor
term.process(b"\x1b[?1002h")     # Button event mouse
term.process(b"\x1b[?1006h")     # SGR mouse encoding
term.process(b"\x1b[?2004h")     # Bracketed paste
term.process(b"\x1b[?2026h")     # Synchronized updates
```

### Minimal Safe Terminal

```python
# Security-first configuration
term = Terminal(cols=80, rows=24, scrollback=0)
# Enable security mode via Python API
# (disable_insecure_sequences and allow_clipboard_read=false by default)
```

### Text Editor / Pager

```python
# Vim-like configuration
term.process(b"\x1b[?1049h")     # Alt screen + save cursor
term.process(b"\x1b[?25h")       # Show cursor
term.process(b"\x1b[?1000h")     # Normal mouse tracking
term.process(b"\x1b[?1006h")     # SGR mouse encoding
```

---

## Environment Variables

The terminal emulator itself does not read environment variables, but host applications typically set:

- `TERM` - Terminal type (e.g., `xterm-256color`)
- `COLORTERM` - True color support indicator (e.g., `truecolor`)
- `TERM_PROGRAM` - Terminal program name
- `TERM_PROGRAM_VERSION` - Version string

---

## Related Documentation

- [README.md](../README.md) - Project overview and complete API reference
- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced features with usage examples
- [VT_FEATURE_PARITY.md](VT_FEATURE_PARITY.md) - Complete VT sequence support matrix
- [SECURITY.md](SECURITY.md) - Security considerations for PTY usage
- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture and design
- [BUILDING.md](BUILDING.md) - Build and installation guide
- [FONTS.md](FONTS.md) - Font support for screenshots
- [examples/](../examples/) - Example scripts and demonstrations
