# API Reference

Complete Python API documentation for par-term-emu-core-rust.

## Table of Contents

- [Terminal Class](#terminal-class)
- [PtyTerminal Class](#ptyterminal-class)
- [Color Utilities](#color-utilities)
- [Data Classes](#data-classes)
  - [Attributes](#attributes)
  - [ShellIntegration](#shellintegration)
  - [Graphic](#graphic)
  - [ScreenSnapshot](#screensnapshot)
  - [NotificationConfig](#notificationconfig)
  - [NotificationEvent](#notificationevent)
  - [RecordingSession](#recordingsession)
  - [Selection](#selection)
  - [ClipboardEntry](#clipboardentry)
  - [ScrollbackStats](#scrollbackstats)
  - [Macro](#macro)
  - [MacroEvent](#macroevent)
- [Enumerations](#enumerations)
  - [CursorStyle](#cursorstyle)
  - [UnderlineStyle](#underlinestyle)

## Terminal Class

The main terminal emulator class for processing ANSI sequences.

### Constructor

```python
Terminal(cols: int, rows: int, scrollback: int = 10000)
```

Create a new terminal with specified dimensions.

**Parameters:**
- `cols`: Number of columns (width)
- `rows`: Number of rows (height)
- `scrollback`: Maximum number of scrollback lines (default: 10000)

### Core Methods

#### Input Processing
- `process(data: bytes)`: Process byte data (can contain ANSI sequences)
- `process_str(text: str)`: Process a string (convenience method)

#### Terminal State
- `content() -> str`: Get terminal content as a string
- `size() -> tuple[int, int]`: Get terminal dimensions (cols, rows)
- `resize(cols: int, rows: int)`: Resize the terminal. When width changes, scrollback content is automatically reflowed (wrapped lines are unwrapped or re-wrapped as needed). All cell attributes are preserved.
- `reset()`: Reset terminal to default state
- `title() -> str`: Get terminal title

#### Cursor Control
- `cursor_position() -> tuple[int, int]`: Get cursor position (col, row)
- `cursor_visible() -> bool`: Check if cursor is visible
- `cursor_style() -> CursorStyle`: Get cursor style (block, underline, bar)
- `cursor_color() -> tuple[int, int, int] | None`: Get cursor color (RGB)
- `set_cursor_style(style: CursorStyle)`: Set cursor style
- `set_cursor_color(r: int, g: int, b: int)`: Set cursor color (RGB)
- `query_cursor_color()`: Query cursor color (response in drain_responses())

#### Keyboard Protocol (Kitty)
- `keyboard_flags() -> int`: Get current Kitty Keyboard Protocol flags
- `set_keyboard_flags(flags: int, mode: int = 1)`: Set flags (mode: 0=disable, 1=set, 2=lock)
- `query_keyboard_flags()`: Query keyboard flags (response in drain_responses())
- `push_keyboard_flags(flags: int)`: Push flags to stack and set new flags
- `pop_keyboard_flags(count: int = 1)`: Pop flags from stack

**Note:** Flags are maintained separately for main and alternate screen buffers with independent stacks. Automatically reset when exiting alternate screen.

#### Clipboard Operations (OSC 52)
- `clipboard() -> str | None`: Get clipboard content
- `set_clipboard(content: str | None)`: Set clipboard content programmatically
- `set_clipboard_with_slot(content: str, slot: str | None = None)`: Set clipboard content for specific slot
- `get_clipboard_from_slot(slot: str | None = None) -> str | None`: Get clipboard content from specific slot
- `allow_clipboard_read() -> bool`: Check if clipboard read is allowed
- `set_allow_clipboard_read(allow: bool)`: Set clipboard read permission (security flag)
- `set_max_clipboard_sync_events(max: int)`: Limit clipboard event history
- `get_max_clipboard_sync_events() -> int`: Get clipboard event limit
- `set_max_clipboard_event_bytes(max: int)`: Truncate large clipboard payloads
- `get_max_clipboard_event_bytes() -> int`: Get clipboard payload size limit

#### Clipboard History
- `add_to_clipboard_history(slot: str, content: str, label: str | None = None)`: Add entry to clipboard history
- `get_clipboard_history(slot: str) -> list[ClipboardEntry]`: Get clipboard history for slot
- `get_latest_clipboard(slot: str) -> ClipboardEntry | None`: Get most recent clipboard entry for slot
- `clear_clipboard_history(slot: str)`: Clear history for specific slot
- `clear_all_clipboard_history()`: Clear all clipboard history
- `search_clipboard_history(pattern: str, slot: str | None = None, case_sensitive: bool = True) -> list[ClipboardEntry]`: Search clipboard history

#### Scrollback Buffer
- `scrollback() -> list[str]`: Get scrollback buffer as list of strings
- `scrollback_len() -> int`: Get number of scrollback lines
- `scrollback_line(index: int) -> list[tuple[char, tuple[int, int, int], tuple[int, int, int], Attributes]] | None`: Get specific scrollback line with full cell data (index 0 = oldest)
- `get_scrollback_usage() -> tuple[int, int]`: Get scrollback usage (used_lines, max_capacity)

#### Cell Inspection
- `get_line(row: int) -> str | None`: Get a specific line
- `get_line_cells(row: int) -> list | None`: Get cells for a specific line with full metadata
- `get_char(col: int, row: int) -> str | None`: Get character at position
- `get_fg_color(col: int, row: int) -> tuple[int, int, int] | None`: Get foreground color (RGB)
- `get_bg_color(col: int, row: int) -> tuple[int, int, int] | None`: Get background color (RGB)
- `get_underline_color(col: int, row: int) -> tuple[int, int, int] | None`: Get underline color (RGB)
- `get_attributes(col: int, row: int) -> Attributes | None`: Get text attributes
- `get_hyperlink(col: int, row: int) -> str | None`: Get hyperlink URL at position (OSC 8)
- `is_line_wrapped(row: int) -> bool`: Check if line is wrapped from previous line

#### Terminal Modes
- `is_alt_screen_active() -> bool`: Check if alternate screen buffer is active
- `bracketed_paste() -> bool`: Check if bracketed paste mode is enabled
- `focus_tracking() -> bool`: Check if focus tracking mode is enabled
- `mouse_mode() -> str`: Get current mouse tracking mode
- `insert_mode() -> bool`: Check if insert mode is enabled
- `line_feed_new_line_mode() -> bool`: Check if line feed/new line mode is enabled
- `synchronized_updates() -> bool`: Check if synchronized updates mode is enabled (DEC 2026)
- `auto_wrap_mode() -> bool`: Check if auto-wrap mode is enabled
- `origin_mode() -> bool`: Check if origin mode (DECOM) is enabled
- `application_cursor() -> bool`: Check if application cursor key mode is enabled

#### VT Conformance Level
- `conformance_level() -> int`: Get current conformance level (1-5 for VT100-VT520)
- `conformance_level_name() -> str`: Get conformance level name ("VT100", "VT220", etc.)
- `set_conformance_level(level: int, c1_mode: int = 2)`: Set conformance level (1-5 or 61-65)

#### Bell Volume Control (VT520)
- `warning_bell_volume() -> int`: Get warning bell volume (0-8)
- `set_warning_bell_volume(volume: int)`: Set warning bell volume (0=off, 1-8=volume levels)
- `margin_bell_volume() -> int`: Get margin bell volume (0-8)
- `set_margin_bell_volume(volume: int)`: Set margin bell volume (0=off, 1-8=volume levels)

#### Scrolling and Margins
- `scroll_region() -> tuple[int, int]`: Get vertical scroll region (top, bottom)
- `left_right_margins() -> tuple[int, int] | None`: Get horizontal margins if set

#### Colors and Appearance
- `default_fg() -> tuple[int, int, int] | None`: Get default foreground color
- `default_bg() -> tuple[int, int, int] | None`: Get default background color
- `set_default_fg(r: int, g: int, b: int)`: Set default foreground color
- `set_default_bg(r: int, g: int, b: int)`: Set default background color
- `query_default_fg()`: Query default foreground color (response in drain_responses())
- `query_default_bg()`: Query default background color (response in drain_responses())
- `get_ansi_color(index: int) -> tuple[int, int, int] | None`: Get ANSI palette color (0-255)
- `get_ansi_palette() -> list[tuple[int, int, int]]`: Get all 16 ANSI colors (indices 0-15)
- `set_ansi_palette_color(index: int, r: int, g: int, b: int)`: Set ANSI palette color (0-255)

#### Shell Integration (OSC 133 & OSC 7)
- `current_directory() -> str | None`: Get current working directory (OSC 7)
- `accept_osc7() -> bool`: Check if OSC 7 (CWD) is accepted
- `set_accept_osc7(accept: bool)`: Set whether to accept OSC 7 sequences
- `shell_integration_state() -> ShellIntegration`: Get shell integration state
- `disable_insecure_sequences() -> bool`: Check if insecure sequences are disabled
- `set_disable_insecure_sequences(disable: bool)`: Disable insecure/dangerous sequences

#### Paste Operations
- `get_paste_start() -> tuple[int, int] | None`: Get bracketed paste start position
- `get_paste_end() -> tuple[int, int] | None`: Get bracketed paste end position
- `paste(text: str)`: Simulate bracketed paste

#### Focus Events
- `get_focus_in_event() -> str`: Get focus-in event sequence
- `get_focus_out_event() -> str`: Get focus-out event sequence

#### Terminal Responses
- `drain_responses() -> list[str]`: Drain all pending terminal responses (DA, DSR, etc.)
- `has_pending_responses() -> bool`: Check if responses are pending

#### Notifications (OSC 9/777)
- `drain_notifications() -> list[tuple[str, str]]`: Drain notifications (title, message)
- `take_notifications() -> list[tuple[str, str]]`: Take notifications without removing
- `has_notifications() -> bool`: Check if notifications are pending
- `set_max_notifications(max: int)`: Limit OSC 9/777 notification backlog
- `get_max_notifications() -> int`: Get notification buffer limit
- `get_notification_config() -> NotificationConfig`: Get current notification configuration
- `set_notification_config(config: NotificationConfig)`: Apply notification configuration
- `trigger_notification(trigger: str, alert: str, message: str | None)`: Manually trigger notification
- `register_custom_trigger(id: int, message: str)`: Register custom notification trigger
- `trigger_custom_notification(id: int, alert: str)`: Trigger custom notification
- `get_notification_events() -> list[NotificationEvent]`: Get notification events
- `mark_notification_delivered(index: int)`: Mark notification as delivered
- `clear_notification_events()`: Clear notification events
- `update_activity()`: Update activity tracking
- `check_silence()`: Check if silence threshold exceeded
- `check_activity()`: Check if activity occurred after inactivity
- `handle_bell_notification()`: Triggers configured bell alerts

#### Graphics
Multi-protocol graphics support: Sixel (DCS), iTerm2 Inline Images (OSC 1337), and Kitty Graphics Protocol (APC G).

- `resize_pixels(width_px: int, height_px: int)`: Resize terminal by pixel dimensions
- `graphics_count() -> int`: Get count of graphics currently displayed
- `graphics_at_row(row: int) -> list[Graphic]`: Get graphics at specific row
- `clear_graphics()`: Clear all graphics
- `graphics_store() -> GraphicsStore`: Get immutable access to graphics store (Rust API only)
- `graphics_store_mut() -> GraphicsStore`: Get mutable access to graphics store (Rust API only)

**Supported Protocols:**
- **Sixel** (DCS): VT340 bitmap graphics via `DCS Pq ... ST`
- **iTerm2** (OSC 1337): Inline images via `OSC 1337 ; File=... ST`
- **Kitty** (APC G): Advanced graphics protocol with image reuse, animation, and Unicode placeholders

**Unicode Placeholders** (Kitty Protocol):
- Virtual placements (`U=1`) insert U+10EEEE placeholder characters in grid
- Metadata encoded in cell colors (image_id in foreground, placement_id in underline)
- Frontend detects placeholders and renders corresponding virtual placement
- Enables inline image display within text flow
- See `src/graphics/placeholder.rs` for encoding details

#### Snapshots
- `create_snapshot() -> ScreenSnapshot`: Create atomic snapshot of current screen state
- `flush_synchronized_updates()`: Flush synchronized updates buffer (DEC 2026)

#### Testing
- `simulate_mouse_event(...)`: Simulate mouse event for testing

#### Export Functions
- `export_text() -> str`: Export entire buffer as plain text without styling
- `export_styled() -> str`: Export entire buffer with ANSI styling
- `export_html(include_styles: bool = True) -> str`: Export as HTML (full document or content only)

#### Screenshots
- `screenshot(format, font_path, font_size, include_scrollback, padding, quality, render_cursor, cursor_color, sixel_mode, scrollback_offset, link_color, bold_color, use_bold_color, minimum_contrast) -> bytes`: Take screenshot and return image bytes
- `screenshot_to_file(path, format, font_path, font_size, include_scrollback, padding, quality, render_cursor, cursor_color, sixel_mode, scrollback_offset, link_color, bold_color, use_bold_color, minimum_contrast)`: Take screenshot and save to file

**Supported Formats:** PNG, JPEG, BMP, SVG (vector), HTML

#### Session Recording
- `start_recording(title: str | None = None)`: Start recording session
- `stop_recording() -> RecordingSession | None`: Stop recording and return session
- `is_recording() -> bool`: Check if recording is active
- `get_recording_session() -> RecordingSession | None`: Get current session info
- `record_output(data: bytes)`: Record output event
- `record_input(data: bytes)`: Record input event
- `record_marker(name: str)`: Add marker/bookmark
- `record_resize(cols: int, rows: int)`: Record resize event
- `export_asciicast(session: RecordingSession | None = None) -> str`: Export to asciicast v2 format
- `export_json(session: RecordingSession | None = None) -> str`: Export to JSON format

### Text Extraction and Selection

#### Text Extraction Utilities
- `get_word_at(col: int, row: int, word_chars: str | None = None) -> str | None`: Extract word at cursor (default word_chars: "/-+\\~_.")
- `get_url_at(col: int, row: int) -> str | None`: Detect and extract URL at cursor
- `get_line_unwrapped(row: int) -> str | None`: Get full logical line following wrapping
- `find_matching_bracket(col: int, row: int) -> tuple[int, int] | None`: Find matching bracket/parenthesis (supports (), [], {}, <>)
- `select_semantic_region(col: int, row: int, delimiters: str) -> str | None`: Extract content between delimiters

#### Selection Management
- `set_selection(start_col: int, start_row: int, end_col: int, end_row: int, mode: str = "character")`: Set text selection (mode: "character", "line", or "block")
- `get_selection() -> Selection | None`: Get current selection
- `get_selected_text() -> str | None`: Get text content of current selection
- `clear_selection()`: Clear current selection
- `select_word_at(col: int, row: int)`: Select word at position
- `select_line(row: int)`: Select entire line

### Content Search

- `find_text(pattern: str, case_sensitive: bool = True) -> list[tuple[int, int]]`: Find all occurrences in visible screen
- `find_next(pattern: str, from_col: int, from_row: int, case_sensitive: bool = True) -> tuple[int, int] | None`: Find next occurrence from position
- `search_scrollback(pattern: str, case_sensitive: bool = True, max_results: int | None = None) -> list[tuple[int, int]]`: Search scrollback buffer

### Buffer Statistics

- `get_stats() -> dict[str, int]`: Get terminal statistics (cols, rows, scrollback_lines, total_cells, non_whitespace_lines, graphics_count, estimated_memory_bytes)
- `count_non_whitespace_lines() -> int`: Count lines containing non-whitespace characters
- `get_scrollback_usage() -> tuple[int, int]`: Get scrollback usage (used_lines, max_capacity)
- `scrollback_stats() -> ScrollbackStats`: Get detailed scrollback statistics

### Static Utility Methods

Call these on the class itself (e.g., `Terminal.strip_ansi(text)`):

- `Terminal.strip_ansi(text: str) -> str`: Remove all ANSI escape sequences from text
- `Terminal.measure_text_width(text: str) -> int`: Measure display width accounting for wide characters and ANSI codes
- `Terminal.parse_color(color_string: str) -> tuple[int, int, int] | None`: Parse color from hex (#RRGGBB), rgb(r,g,b), or name

## PtyTerminal Class

Terminal emulator with PTY (pseudo-terminal) support for interactive shell sessions.

### Constructor

```python
PtyTerminal(cols: int, rows: int, scrollback: int = 10000)
```

**Inherits:** All methods from `Terminal` class

### PTY-Specific Methods

#### Process Management
- `spawn(cmd: str, args: list[str] = [], env: dict[str, str] | None = None, cwd: str | None = None)`: Spawn a command with arguments
- `spawn_shell(shell: str | None = None)`: Spawn a shell (defaults to /bin/bash)
- `is_running() -> bool`: Check if the child process is still running
- `wait() -> int | None`: Wait for child process to exit and return exit code
- `try_wait() -> int | None`: Non-blocking check if child has exited
- `kill()`: Forcefully terminate the child process
- `get_default_shell() -> str`: Get the default shell path

#### I/O Operations
- `write(data: bytes)`: Write bytes to the PTY
- `write_str(text: str)`: Write string to the PTY (convenience method)

#### Update Tracking
- `update_generation() -> int`: Get current update generation counter
- `has_updates_since(generation: int) -> bool`: Check if terminal updated since generation
- `send_resize_pulse()`: Send SIGWINCH to child process after resize
- `bell_count() -> int`: Get bell event count (increments on BEL/\\x07)

#### Appearance Settings (PTY-Specific)
- `set_bold_brightening(enabled: bool)`: Enable/disable bold brightening (ANSI colors 0-7 → 8-15)

**Note:** PtyTerminal inherits all Terminal methods, so you can also use all Terminal appearance settings like `set_default_fg()`, `set_default_bg()`, etc.

### Context Manager Support

```python
with PtyTerminal(80, 24) as term:
    term.spawn_shell()
    term.write_str("echo 'Hello'\n")
    # Automatic cleanup on exit
```

## Color Utilities

Comprehensive color manipulation functions available as standalone module functions.

### Brightness and Contrast

- `perceived_brightness_rgb(r: int, g: int, b: int) -> float`: Calculate perceived brightness (0.0-1.0) using NTSC formula (30% red, 59% green, 11% blue)
- `adjust_contrast_rgb(fg: tuple[int, int, int], bg: tuple[int, int, int], min_contrast: float) -> tuple[int, int, int]`: Adjust foreground for minimum contrast ratio (0.0-1.0), preserving hue

### Basic Adjustments

- `lighten_rgb(rgb: tuple[int, int, int], amount: float) -> tuple[int, int, int]`: Lighten color by percentage (0.0-1.0)
- `darken_rgb(rgb: tuple[int, int, int], amount: float) -> tuple[int, int, int]`: Darken color by percentage (0.0-1.0)

### Accessibility (WCAG)

- `color_luminance(rgb: tuple[int, int, int]) -> float`: Calculate relative luminance (0.0-1.0) per WCAG formula
- `is_dark_color(rgb: tuple[int, int, int]) -> bool`: Check if color is dark (luminance < 0.5)
- `contrast_ratio(fg: tuple[int, int, int], bg: tuple[int, int, int]) -> float`: Calculate WCAG contrast ratio (1.0-21.0)
- `meets_wcag_aa(fg: tuple[int, int, int], bg: tuple[int, int, int]) -> bool`: Check if contrast meets WCAG AA (4.5:1)
- `meets_wcag_aaa(fg: tuple[int, int, int], bg: tuple[int, int, int]) -> bool`: Check if contrast meets WCAG AAA (7:1)

### Color Mixing and Manipulation

- `mix_colors(color1: tuple[int, int, int], color2: tuple[int, int, int], ratio: float) -> tuple[int, int, int]`: Mix two colors (ratio: 0.0=color1, 1.0=color2)
- `complementary_color(rgb: tuple[int, int, int]) -> tuple[int, int, int]`: Get complementary color (opposite on color wheel)

### Color Space Conversions

- `rgb_to_hsl(rgb: tuple[int, int, int]) -> tuple[int, int, int]`: Convert RGB to HSL (H: 0-360, S: 0-100, L: 0-100)
- `hsl_to_rgb(h: int, s: int, l: int) -> tuple[int, int, int]`: Convert HSL to RGB
- `rgb_to_hex(rgb: tuple[int, int, int]) -> str`: Convert RGB to hex string (#RRGGBB)
- `hex_to_rgb(hex_str: str) -> tuple[int, int, int]`: Convert hex string to RGB
- `rgb_to_ansi_256(rgb: tuple[int, int, int]) -> int`: Find nearest ANSI 256-color palette index (0-255)

### Advanced Adjustments

- `adjust_saturation(rgb: tuple[int, int, int], amount: int) -> tuple[int, int, int]`: Adjust saturation by amount (-100 to +100)
- `adjust_hue(rgb: tuple[int, int, int], degrees: int) -> tuple[int, int, int]`: Shift hue by degrees (0-360)

## Data Classes

### Attributes

Represents text attributes for a cell.

**Properties:**
- `bold: bool`: Bold text
- `dim: bool`: Dim text
- `italic: bool`: Italic text
- `underline: bool`: Underlined text
- `blink: bool`: Blinking text
- `reverse: bool`: Reverse video
- `hidden: bool`: Hidden text
- `strikethrough: bool`: Strikethrough text

### ShellIntegration

Shell integration state (OSC 133 and OSC 7).

**Properties:**
- `in_prompt: bool`: True if currently in prompt (marker A)
- `in_command_input: bool`: True if currently in command input (marker B)
- `in_command_output: bool`: True if currently in command output (marker C)
- `current_command: str | None`: The command that was executed
- `last_exit_code: int | None`: Exit code from last command (marker D)
- `cwd: str | None`: Current working directory from OSC 7

### Graphic

Sixel graphic metadata.

**Properties:**
- `row: int`: Display row
- `col: int`: Display column
- `width: int`: Width in pixels
- `height: int`: Height in pixels
- `data: bytes`: Image data

### ScreenSnapshot

Immutable snapshot of screen state.

**Methods:**
- `content() -> str`: Get full screen content
- `cursor_position() -> tuple[int, int]`: Cursor position at snapshot time
- `size() -> tuple[int, int]`: Terminal dimensions

### NotificationConfig

Notification configuration settings.

**Properties:**
- `bell_desktop: bool`: Enable desktop notifications on bell
- `bell_sound: int`: Bell sound volume (0-100, 0=disabled)
- `bell_visual: bool`: Enable visual alert on bell
- `activity_enabled: bool`: Enable activity notifications
- `activity_threshold: int`: Activity threshold in seconds
- `silence_enabled: bool`: Enable silence notifications
- `silence_threshold: int`: Silence threshold in seconds

### NotificationEvent

Notification event information.

**Properties:**
- `trigger: str`: Trigger type (Bell, Activity, Silence, Custom)
- `alert: str`: Alert type (Desktop, Sound, Visual)
- `message: str | None`: Notification message
- `delivered: bool`: Whether notification was delivered
- `timestamp: int`: Event timestamp (Unix timestamp in seconds)

### RecordingSession

Session recording metadata.

**Properties:**
- `start_time: int`: Recording start timestamp (milliseconds)
- `initial_size: tuple[int, int]`: Initial terminal dimensions (cols, rows)
- `duration: int`: Recording duration in milliseconds
- `event_count: int`: Number of recorded events
- `title: str | None`: Session title

**Methods:**
- `get_size() -> tuple[int, int]`: Get recording size (cols, rows)
- `get_duration_seconds() -> float`: Get recording duration in seconds

### Selection

Text selection information.

**Properties:**
- `start: tuple[int, int]`: Selection start position (col, row)
- `end: tuple[int, int]`: Selection end position (col, row)
- `mode: str`: Selection mode ("character", "line", or "block")

### ClipboardEntry

Clipboard history entry.

**Properties:**
- `content: str`: Clipboard content
- `timestamp: int`: Entry timestamp (Unix timestamp in seconds)
- `label: str | None`: Optional label for the entry

### ScrollbackStats

Scrollback buffer statistics.

**Properties:**
- `total_lines: int`: Total scrollback lines
- `memory_bytes: int`: Estimated memory usage in bytes
- `has_wrapped: bool`: Whether the scrollback buffer has wrapped (cycled)

### Macro

Macro recording for keyboard automation.

**Properties:**
- `name: str`: Macro name
- `duration: int`: Total duration in milliseconds
- `events: list[MacroEvent]`: List of macro events

**Methods:**
- `add_key(key: str)`: Add a key press event
- `add_delay(duration: int)`: Add a delay event
- `add_screenshot(label: str | None = None)`: Add a screenshot trigger event
- `to_yaml() -> str`: Export macro to YAML format
- `from_yaml(yaml_str: str) -> Macro`: Load macro from YAML format (static method)

### MacroEvent

Event in a macro recording.

**Properties:**
- `event_type: str`: Event type ("key", "delay", or "screenshot")
- `timestamp: int`: Event timestamp in milliseconds
- `key: str | None`: Key name for key press events
- `duration: int | None`: Duration in milliseconds for delay events
- `label: str | None`: Label for screenshot events

## Enumerations

### CursorStyle

Cursor display styles (DECSCUSR).

**Values:**
- `CursorStyle.BlinkingBlock`: Blinking block cursor (default)
- `CursorStyle.SteadyBlock`: Steady block cursor
- `CursorStyle.BlinkingUnderline`: Blinking underline cursor
- `CursorStyle.SteadyUnderline`: Steady underline cursor
- `CursorStyle.BlinkingBar`: Blinking bar/I-beam cursor
- `CursorStyle.SteadyBar`: Steady bar/I-beam cursor

### UnderlineStyle

Text underline styles.

**Values:**
- `UnderlineStyle.None_`: No underline
- `UnderlineStyle.Straight`: Straight underline (default)
- `UnderlineStyle.Double`: Double underline
- `UnderlineStyle.Curly`: Curly underline (for spell check)
- `UnderlineStyle.Dotted`: Dotted underline
- `UnderlineStyle.Dashed`: Dashed underline

## See Also

- [VT Sequences Reference](VT_SEQUENCES.md) - Complete list of supported ANSI/VT sequences
- [Advanced Features](ADVANCED_FEATURES.md) - Detailed feature documentation
- [Examples](../examples/) - Usage examples and demonstrations
