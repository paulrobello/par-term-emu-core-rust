# Terminal Core Configuration Reference

Comprehensive reference for par-term-emu-core-rust terminal emulator configuration options, modes, and settings.

This document covers the **terminal core configuration** (Rust library internals) for the par-term-emu terminal emulator. For TUI application settings, see the [par-term-emu-tui-rust project](https://github.com/paulrobello/par-term-emu-tui-rust).

## Table of Contents

- [Overview](#overview)
- [Terminal Construction](#terminal-construction)
- [Runtime Configuration](#runtime-configuration)
- [Terminal Modes](#terminal-modes)
- [Core Mouse Configuration](#core-mouse-configuration)
- [Core Security Settings](#core-security-settings)
- [Sixel Resource Limits](#sixel-resource-limits)
- [Keyboard Protocol](#keyboard-protocol)
- [Color Configuration](#color-configuration)
- [Screenshot Configuration](#screenshot-configuration)
- [Terminal Notification Configuration](#terminal-notification-configuration)
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
- **TUI Layer**: Application-level settings for the TUI (see [par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust))

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
from par_term_emu_core_rust import Terminal

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
| Attribute Change Extent | `CSI Ps * x` (DECSACE) | `2` | 0/1: stream mode, 2: rectangle mode (default) |

**Notes:**
- DECSACE controls how SGR attributes apply in rectangular operations
- Stream mode (0/1): attributes change affects character stream
- Rectangle mode (2): attributes apply only within rectangle bounds

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

### Advanced VT Settings

These settings control VT conformance and terminal bell behavior:

| Setting | Type | Default | VT Sequence | Description |
|---------|------|---------|-------------|-------------|
| Conformance Level | `u16` | VT520 | `CSI 6x ; y " p` | VT100/VT220/VT320/VT420/VT520 conformance level |
| Warning Bell Volume | `u8` | 4 | `CSI Ps SP t` (DECSWBV) | Volume for warning bells (0=off, 1-8=volume) |
| Margin Bell Volume | `u8` | 4 | `CSI Ps SP u` (DECSMBV) | Volume for margin bells (0=off, 1-8=volume) |

**Python API:**
```python
# Set conformance level
term.set_conformance_level(level=520, c1_mode=0)  # VT520, 7-bit C1 controls

# Configure bell volumes
term.set_warning_bell_volume(5)  # Medium volume
term.set_margin_bell_volume(3)   # Low volume
```

**Notes:**
- Conformance level affects which VT features are available
- Higher levels include features from lower levels
- C1 mode: 0 = 7-bit, 1 = 8-bit control characters
- Bell volumes are legacy VT features, separate from NotificationConfig

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

## Sixel Resource Limits

Sixel graphics are subject to per-terminal resource limits to prevent
pathological memory usage from malicious or malformed input.

### Defaults and Hard Ceilings

- **Default per-terminal limits:**
  - `max_width_px` = 1024
  - `max_height_px` = 1024
  - `max_repeat` (for `!Pn` sequences) = 10_000
  - `max_graphics` (in-memory Sixel graphics) = 256
- **Hard ceilings (enforced in Rust):**
  - `max_width_px` ≤ 4096
  - `max_height_px` ≤ 4096
  - `max_repeat` ≤ 10_000
  - `max_graphics` ≤ 1024

These limits are applied when parsing Sixel DCS sequences and when constructing
the final in-memory bitmap for screenshots or rendering.

### API Control

You can query and adjust limits at runtime:

- Rust (`Terminal`):
  - `fn sixel_limits(&self) -> SixelLimits`
  - `fn set_sixel_limits(&mut self, max_width: usize, max_height: usize, max_repeat: usize)`
  - `fn max_sixel_graphics(&self) -> usize`
  - `fn set_max_sixel_graphics(&mut self, max_graphics: usize)`
    - Values are clamped into the safe range `[1, HARD_MAX]`.

- Python (`Terminal` and `PtyTerminal`):

  ```python
  max_w, max_h, max_repeat = term.get_sixel_limits()
  term.set_sixel_limits(512, 512, 2000)
  term.set_sixel_graphics_limit(128)
  stats = term.get_sixel_stats()
  ```

Use tighter limits (e.g. 512x512) when displaying untrusted Sixel content, and
only relax them in trusted environments where large images are expected.

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

| Flag | Type | Default | Python API | Description |
|------|------|---------|------------|-------------|
| `use_bold_color` | `bool` | `false` | `set_use_bold_color(bool)` | Use custom bold color instead of bright variant |
| `use_underline_color` | `bool` | `false` | `set_use_underline_color(bool)` | Use custom underline color (SGR 58) |
| `use_cursor_guide` | `bool` | `false` | Not exposed | Show cursor guide (column/row highlight) |
| `use_selected_text_color` | `bool` | `false` | Not exposed | Use custom selection text color |
| `smart_cursor_color` | `bool` | `false` | Not exposed | Auto-adjust cursor color based on background |
| `bold_brightening` | `bool` | `true` | Not exposed in Python | Bold ANSI colors 0-7 brighten to 8-15 |

**Notes:**
- These settings provide feature parity with iTerm2's color configuration
- Colors can be queried via OSC sequences (10, 11, 12, etc.)
- Custom colors only apply when corresponding `use_*` flags are enabled
- Bold brightening defaults to `true` in Terminal core (iTerm2 compatibility) but `false` in ScreenshotConfig
- Some flags (`use_cursor_guide`, `use_selected_text_color`, `smart_cursor_color`, `bold_brightening`) are currently only accessible via Rust API (not exposed in Python bindings for runtime terminal configuration)

---

## Screenshot Configuration

**Note**: This section describes the **core library** screenshot API. For TUI application screenshot settings (like `screenshot_directory`, `screenshot_format`, `open_screenshot_after_capture`), see the [par-term-emu-tui-rust project](https://github.com/paulrobello/par-term-emu-tui-rust).

### Overview

The terminal core provides programmatic screenshot capabilities via Python and Rust APIs. Screenshots can be taken in multiple formats with extensive configuration options for fonts, colors, rendering modes, and content selection.

### Supported Formats

| Format | Description | Use Case |
|--------|-------------|----------|
| `png` | PNG (lossless raster) | General purpose, default format |
| `jpeg`/`jpg` | JPEG (lossy raster) | Smaller file size, configurable quality |
| `bmp` | BMP (uncompressed raster) | Maximum compatibility, large files |
| `svg` | SVG (vector graphics) | Scalable, text-selectable output |

**Note**: HTML export is available via the separate `export_html(include_styles: bool)` method, not the screenshot API.

### Screenshot Configuration Options

#### Font Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `font_path` | `Option<String>` | `None` | Path to custom TTF/OTF font (None = embedded JetBrains Mono) |
| `font_size` | `f32` | `14.0` | Font size in pixels |
| `line_height_multiplier` | `f32` | `1.2` | Line height multiplier (1.0 = tight, 1.2 = comfortable) |
| `char_width_multiplier` | `f32` | `1.0` | Character width multiplier for spacing |
| `antialiasing` | `bool` | `true` | Enable font antialiasing (raster formats only) |

#### Content Selection

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `include_scrollback` | `bool` | `false` | Include scrollback buffer in screenshot |
| `scrollback_lines` | `Option<usize>` | `None` | Number of scrollback lines to include (None = all) |

**Note:** The `scrollback_offset` parameter is passed separately to the `screenshot()` method, not via `ScreenshotConfig`.

#### Canvas Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `padding_px` | `u32` | `10` | Padding around content in pixels |
| `background_color` | `Option<(u8,u8,u8)>` | `None` | Background color RGB (None = use terminal default) |

#### Output Format

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `format` | `str` | `"png"` | Image format ("png", "jpeg"/"jpg", "bmp", "svg") |
| `quality` | `u8` | `90` | JPEG quality (1-100, only applies to JPEG format) |

#### Cursor Rendering

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `render_cursor` | `bool` | `false` | Render cursor in screenshot |
| `cursor_color` | `(u8,u8,u8)` | `(255,255,255)` | Cursor color RGB |

#### Sixel Graphics

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `sixel_render_mode` | `str` | `"halfblocks"` | Sixel rendering mode ("disabled", "pixels", "halfblocks") |

**Sixel Render Modes:**
- `disabled` - Don't render Sixel graphics
- `pixels` - Render as actual pixels (shows real image data)
- `halfblocks` - Render using half-block characters (matches TUI appearance)

#### Theme Settings

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `link_color` | `Option<(u8,u8,u8)>` | `None` | Hyperlink color RGB (None = use terminal default) |
| `bold_color` | `Option<(u8,u8,u8)>` | `None` | Bold text color RGB (None = use terminal default) |
| `use_bold_color` | `bool` | `false` | Use custom bold color instead of bright variant |
| `bold_brightening` | `bool` | `false` | Bold ANSI colors 0-7 brighten to 8-15 |
| `minimum_contrast` | `f64` | `0.5` | Minimum contrast adjustment (0.0-1.0, iTerm2-compatible) |
| `faint_text_alpha` | `f32` | `0.5` | Alpha multiplier for faint/dim text (0.0-1.0) |

**Minimum Contrast:**
- `0.0` - Disabled (use colors as-is)
- `0.5` - Moderate contrast (default, matches iTerm2 • 50%)
- `1.0` - Maximum contrast (ensures all text is readable)

**Faint Text Alpha:**
- Controls opacity of faint/dim text (SGR 2)
- `0.0` - Fully transparent (invisible)
- `0.5` - 50% opacity (default, matches iTerm2)
- `1.0` - No dimming (full opacity)

**Note:** In ScreenshotConfig, `use_bold_color` and `bold_brightening` are boolean values (not `Option<bool>`). When not specified in the screenshot API, they use the config defaults (false for both).

### Python API Examples

```python
from par_term_emu_core_rust import Terminal

term = Terminal(cols=80, rows=24)
term.process_str("\x1b[31mRed text\x1b[0m\n")

# Basic screenshot (PNG with defaults)
png_bytes = term.screenshot()

# JPEG with custom quality
jpg_bytes = term.screenshot(format="jpeg", quality=85)

# SVG with custom font
svg_bytes = term.screenshot(
    format="svg",
    font_path="/path/to/font.ttf",
    font_size=16.0
)

# Screenshot with theme settings
# Note: bold_brightening and use_bold_color are booleans (not Optional)
png_bytes = term.screenshot(
    bold_brightening=True,
    use_bold_color=True,
    background_color=(32, 32, 32),
    link_color=(100, 149, 237),
    bold_color=(255, 255, 255),
    minimum_contrast=0.7,
    faint_text_alpha=0.6
)

# Capture scrollback content
png_bytes = term.screenshot(
    include_scrollback=True,
    scrollback_lines=100  # Last 100 lines from scrollback
)

# Capture historical view (scroll back 10 lines from current position)
# Note: scrollback_offset is a method parameter, not a config option
png_bytes = term.screenshot(scrollback_offset=10)

# Save directly to file (format is auto-detected from extension, or specify explicitly)
term.screenshot_to_file(
    path="/path/to/output.png",
    format=None,  # Optional, auto-detected from extension if None
    padding=20,
    font_size=18.0
)

# Sixel graphics rendering
png_bytes = term.screenshot(sixel_mode="pixels")  # Show actual image data
```

### Implementation Details

**Location:**
- Configuration: `src/screenshot/config.rs` - `ScreenshotConfig` struct
- Python bindings: `src/python_bindings/terminal.rs` - `screenshot()` and `screenshot_to_file()` methods
- Renderer: `src/screenshot/renderer.rs` - Core rendering logic

**Font Support:**
- **Embedded fonts**: JetBrains Mono (primary) + Noto Emoji (monochrome)
- **System fallback**: Automatically uses system emoji and CJK fonts when available
- **Custom fonts**: Specify via `font_path` parameter (TTF/OTF formats)
- **Programming ligatures**: Supported in embedded JetBrains Mono font

**Color Processing:**
- Minimum contrast uses NTSC perceived brightness formula (30% red, 59% green, 11% blue)
- Contrast adjustment preserves hue while adjusting brightness
- iTerm2-compatible implementation for feature parity

**Related Features:**
- `export_text()` - Export as plain text (strips all formatting and ANSI codes)
- `export_styled()` - Export with ANSI escape sequences for colors and text attributes
- `export_html(include_styles)` - Export as HTML (with optional inline CSS styles)

---

## Terminal Notification Configuration

Terminal supports comprehensive notification features for various events. This configuration is separate from the notification content itself (OSC 9/777 sequences).

### NotificationConfig Structure

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `bell_desktop` | `bool` | `false` | Enable desktop notifications on bell (BEL/\x07) |
| `bell_sound` | `u8` | `0` | Bell sound volume (0 = disabled, 1-100 = volume level) |
| `bell_visual` | `bool` | `true` | Enable visual alert on bell (flash screen) |
| `activity_enabled` | `bool` | `false` | Enable notifications when terminal becomes active after inactivity |
| `activity_threshold` | `u64` | `10` | Activity threshold in seconds (inactivity before triggering) |
| `silence_enabled` | `bool` | `false` | Enable notifications when terminal becomes silent after activity |
| `silence_threshold` | `u64` | `300` | Silence threshold in seconds (activity before silence notification) |

### Python API

```python
# Get current notification configuration
config = term.get_notification_config()
print(f"Bell desktop: {config.bell_desktop}")
print(f"Bell sound: {config.bell_sound}")

# Set notification configuration
# Note: NotificationConfig must be imported from the private _native module
from par_term_emu_core_rust._native import NotificationConfig

config = NotificationConfig()
config.bell_desktop = True
config.bell_sound = 50
config.bell_visual = True
config.activity_enabled = True
config.activity_threshold = 30
config.silence_enabled = True
config.silence_threshold = 600
term.set_notification_config(config)

# Manually trigger notifications
term.trigger_notification("Bell", "Desktop", "Custom bell message")
term.trigger_notification("Activity", "Sound(75)", None)

# Register and trigger custom notifications
term.register_custom_trigger(1, "Build completed")
term.trigger_custom_notification(1, "Desktop")

# Monitor notification events
events = term.get_notification_events()
for event in events:
    print(f"Trigger: {event.trigger}, Alert: {event.alert}")

# Clear notification history
term.clear_notification_events()

# Configure max retained OSC 9/777 notifications
term.set_max_notifications(100)
max_notifs = term.get_max_notifications()
```

### Notification Types

**Triggers:**
- `Bell` - Terminal bell (BEL/\x07)
- `Activity` - Terminal becomes active after inactivity period
- `Silence` - Terminal becomes silent after activity period
- `Custom(id)` - User-defined custom triggers

**Alerts:**
- `Desktop` - System desktop notification
- `Sound(volume)` - Audio notification with volume (0-100)
- `Visual` - Visual flash/alert in terminal

### Usage Notes

- Notification configuration is checked on each event (bell, activity check, silence check)
- Activity/silence detection requires manual polling via `check_activity()` / `check_silence()`
- Desktop notifications require OS support and permissions
- Visual alerts are always enabled by default for accessibility
- Custom triggers allow application-specific notifications

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

**Note**: The following configuration example is for the **TUI application** ([par-term-emu-tui-rust](https://github.com/paulrobello/par-term-emu-tui-rust)), not the core library. Core library configuration is done programmatically via API calls.

**Complete TUI Configuration Example** (`~/.config/par-term-emu/config.yaml`):

```yaml
# ~/.config/par-term-emu/config.yaml

# --- Selection & Clipboard ---
auto_copy_selection: true                 # Auto-copy on select
keep_selection_after_copy: true           # Keep highlight after copy
expose_system_clipboard: true             # OSC 52 access
copy_trailing_newline: true               # Include \n when copying lines
word_characters: "/-+\\~_." # Word boundary characters (iTerm2-compatible default)
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
screenshot_format: "png"                  # Format: png, jpeg, bmp, svg
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
- [VT_TECHNICAL_REFERENCE.md](VT_TECHNICAL_REFERENCE.md) - Complete VT sequence support matrix and implementation details
- [SECURITY.md](SECURITY.md) - Security considerations for PTY usage
- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture and design
- [BUILDING.md](BUILDING.md) - Build and installation guide
- [FONTS.md](FONTS.md) - Font support for screenshots
- [examples/](../examples/) - Example scripts and demonstrations
