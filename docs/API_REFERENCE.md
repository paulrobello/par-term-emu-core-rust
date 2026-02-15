# API Reference

Complete Python API documentation for par-term-emu-core-rust.

## Table of Contents

- [Terminal Class](#terminal-class)
  - [Core Methods](#core-methods)
  - [Terminal State](#terminal-state)
  - [Cursor Control](#cursor-control)
  - [Keyboard Protocol](#keyboard-protocol-kitty)
  - [Clipboard Operations](#clipboard-operations-osc-52)
  - [Clipboard History](#clipboard-history)
  - [Scrollback Buffer](#scrollback-buffer)
  - [Cell Inspection](#cell-inspection)
  - [Terminal Modes](#terminal-modes)
  - [VT Conformance Level](#vt-conformance-level)
  - [Bell Volume Control](#bell-volume-control-vt520)
  - [Scrolling and Margins](#scrolling-and-margins)
  - [Colors and Appearance](#colors-and-appearance)
  - [Theme Colors](#theme-colors)
  - [Text Rendering Options](#text-rendering-options)
  - [Shell Integration](#shell-integration-osc-133--osc-7)
  - [Paste Operations](#paste-operations)
  - [Focus Events](#focus-events)
  - [Terminal Responses](#terminal-responses)
  - [Notifications](#notifications-osc-9777)
  - [Graphics](#graphics)
  - [Snapshots](#snapshots)
  - [Testing](#testing)
  - [Export Functions](#export-functions)
  - [Screenshots](#screenshots)
  - [Session Recording](#session-recording)
  - [Advanced Search and Regex](#advanced-search-and-regex)
  - [Mouse Tracking and Events](#mouse-tracking-and-events)
  - [Bookmarks](#bookmarks)
  - [Triggers & Automation](#triggers--automation)
  - [Shell Integration Extended](#shell-integration-extended)
  - [Semantic Zones](#semantic-zones)
  - [Semantic Snapshot](#semantic-snapshot)
  - [Clipboard Extended](#clipboard-extended)
  - [Graphics Extended](#graphics-extended)
  - [File Transfer](#file-transfer)
  - [Rendering and Damage Tracking](#rendering-and-damage-tracking)
  - [Performance and Benchmarking](#performance-and-benchmarking)
  - [Tmux Control Mode](#tmux-control-mode)
  - [Session Management](#session-management)
  - [Advanced Text Operations](#advanced-text-operations)
  - [Testing and Compliance](#testing-and-compliance)
  - [Unicode Normalization](#unicode-normalization)
  - [Utility Methods](#utility-methods)
  - [Debug and Snapshot Methods](#debug-and-snapshot-methods)
  - [Text Extraction and Selection](#text-extraction-and-selection)
  - [Content Search](#content-search)
  - [Buffer Statistics](#buffer-statistics)
  - [Static Utility Methods](#static-utility-methods)
  - [Observer API](#observer-api)
- [Observer Convenience Functions](#observer-convenience-functions)
- [PtyTerminal Class](#ptyterminal-class)
  - [Process Management](#process-management)
  - [I/O Operations](#io-operations)
  - [Update Tracking](#update-tracking)
  - [Appearance Settings](#appearance-settings-pty-specific)
  - [Macro Playback](#macro-playback-pty-specific)
  - [Coprocess Management](#coprocess-management)
  - [Context Manager Support](#context-manager-support)
- [Color Utilities](#color-utilities)
- [Data Classes](#data-classes)
  - [Attributes](#attributes)
  - [ShellIntegration](#shellintegration)
  - [Graphic](#graphic)
  - [ImagePlacement](#imageplacement)
  - [ImageDimension](#imagedimension)
  - [ScreenSnapshot](#screensnapshot)
  - [NotificationConfig](#notificationconfig)
  - [NotificationEvent](#notificationevent)
  - [RecordingSession](#recordingsession)
  - [Selection](#selection)
  - [ClipboardEntry](#clipboardentry)
  - [ScrollbackStats](#scrollbackstats)
  - [Macro](#macro)
  - [MacroEvent](#macroevent)
  - [BenchmarkResult](#benchmarkresult)
  - [BenchmarkSuite](#benchmarksuite)
  - [ComplianceTest](#compliancetest)
  - [ComplianceReport](#compliancereport)
  - [CommandExecution](#commandexecution)
  - [CwdChange](#cwdchange)
  - [DamageRegion](#damageregion)
  - [DetectedItem](#detecteditem)
  - [EscapeSequenceProfile](#escapesequenceprofile)
  - [FrameTiming](#frametiming)
  - [ImageProtocol](#imageprotocol)
  - [ImageFormat](#imageformat)
  - [InlineImage](#inlineimage)
  - [JoinedLines](#joinedlines)
  - [LineDiff](#linediff)
  - [MouseEncoding](#mouseencoding)
  - [MouseEvent](#mouseevent)
  - [MousePosition](#mouseposition)
  - [PaneState](#panestate)
  - [PerformanceMetrics](#performancemetrics)
  - [ProfilingData](#profilingdata)
  - [RegexMatch](#regexmatch)
  - [RenderingHint](#renderinghint)
  - [SessionState](#sessionstate)
  - [ShellIntegrationStats](#shellintegrationstats)
  - [SnapshotDiff](#snapshotdiff)
  - [TmuxNotification](#tmuxnotification)
  - [Trigger](#trigger)
  - [TriggerAction](#triggeraction)
  - [TriggerMatch](#triggermatch)
  - [NormalizationForm](#normalizationform)
  - [CoprocessConfig](#coprocessconfig)
  - [WindowLayout](#windowlayout)
  - [ColorHSL](#colorhsl)
  - [ColorHSV](#colorhsv)
  - [ColorPalette](#colorpalette)
  - [Bookmark](#bookmark)
  - [ClipboardHistoryEntry](#clipboardhistoryentry)
  - [ClipboardSyncEvent](#clipboardsyncevent)
  - [SearchMatch](#searchmatch)
- [Enumerations](#enumerations)
  - [CursorStyle](#cursorstyle)
  - [UnderlineStyle](#underlinestyle)
  - [ProgressState](#progressstate)
- [StreamingServer Class](#streamingserver-class)
- [StreamingConfig Class](#streamingconfig-class)
- [Streaming Functions](#streaming-functions)
  - [encode_server_message](#encode_server_message)
  - [decode_server_message](#decode_server_message)
- [Instant Replay](#instant-replay)
  - [Snapshot Capture (Rust)](#snapshot-capture-rust)
  - [SnapshotManager](#snapshotmanager)
  - [ReplaySession](#replaysession)
  - [Python Binding](#instant-replay-python-binding)

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

#### Badge Format (OSC 1337 SetBadgeFormat)
- `badge_format() -> str | None`: Get current badge format template
- `set_badge_format(format: str | None)`: Set badge format template with `\(variable)` placeholders
- `clear_badge_format()`: Clear badge format
- `evaluate_badge() -> str | None`: Evaluate badge format with session variables
- `get_badge_session_variable(name: str) -> str | None`: Get a session variable by name
- `set_badge_session_variable(name: str, value: str)`: Set a custom session variable
- `get_badge_session_variables() -> dict[str, str]`: Get all session variables

**Built-in Variables:** `hostname`, `username`, `path`, `job`, `last_command`, `profile_name`, `tty`, `columns`, `rows`, `bell_count`, `selection`, `tmux_pane_title`, `session_name`, `title`

**Example:**
```python
term.set_badge_format(r"\(username)@\(hostname)")
term.set_badge_session_variable("username", "alice")
term.set_badge_session_variable("hostname", "server1")
badge = term.evaluate_badge()  # "alice@server1"
```

#### User Variables (OSC 1337 SetUserVar)
- `get_user_var(name: str) -> str | None`: Get a user variable set via OSC 1337 SetUserVar
- `get_user_vars() -> dict[str, str]`: Get all user variables as a dictionary

User variables are set automatically when the terminal processes `OSC 1337 ; SetUserVar=<name>=<base64_value> ST` sequences from shell integration scripts. They are also accessible as badge session variables.

**Example:**
```python
# After shell sends SetUserVar sequences:
host = term.get_user_var("hostname")   # "server1"
user = term.get_user_var("username")   # "alice"
all_vars = term.get_user_vars()        # {"hostname": "server1", "username": "alice"}
```

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

#### modifyOtherKeys (XTerm Extension)
- `modify_other_keys_mode() -> int`: Get current mode (0=disabled, 1=special keys, 2=all keys)
- `set_modify_other_keys_mode(mode: int)`: Set mode directly (values > 2 clamped to 2)

**Sequences:**
- `CSI > 4 ; mode m` - Set mode via escape sequence
- `CSI ? 4 m` - Query mode (response: `CSI > 4 ; mode m` in drain_responses())

**Note:** Mode resets to 0 on terminal reset and when exiting alternate screen.

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

#### Theme Colors
- `link_color() -> tuple[int, int, int]`: Get hyperlink color (OSC 8)
- `set_link_color(r: int, g: int, b: int)`: Set hyperlink color
- `bold_color() -> tuple[int, int, int]`: Get bold text color
- `set_bold_color(r: int, g: int, b: int)`: Set bold text color
- `cursor_guide_color() -> tuple[int, int, int]`: Get cursor guide/column color
- `set_cursor_guide_color(r: int, g: int, b: int)`: Set cursor guide color
- `badge_color() -> tuple[int, int, int]`: Get badge/notification color
- `set_badge_color(r: int, g: int, b: int)`: Set badge color
- `match_color() -> tuple[int, int, int]`: Get search match highlight color
- `set_match_color(r: int, g: int, b: int)`: Set search match color
- `selection_bg_color() -> tuple[int, int, int]`: Get selection background color
- `set_selection_bg_color(r: int, g: int, b: int)`: Set selection background color
- `selection_fg_color() -> tuple[int, int, int]`: Get selection foreground/text color
- `set_selection_fg_color(r: int, g: int, b: int)`: Set selection foreground color

#### Text Rendering Options
- `use_bold_color() -> bool`: Check if custom bold color is used instead of bright ANSI variant
- `set_use_bold_color(use_bold: bool)`: Enable/disable custom bold color
- `use_underline_color() -> bool`: Check if custom underline color is enabled
- `set_use_underline_color(use_underline: bool)`: Enable/disable custom underline color
- `bold_brightening() -> bool`: Check if bold text with colors 0-7 is brightened to 8-15
- `set_bold_brightening(enabled: bool)`: Enable/disable bold brightening (legacy behavior)
- `faint_text_alpha() -> float`: Get alpha multiplier for SGR 2 (dim/faint) text (0.0-1.0, default 0.5)
- `set_faint_text_alpha(alpha: float)`: Set alpha multiplier for dim text (clamped to 0.0-1.0)

#### Shell Integration (OSC 133 & OSC 7)
- `current_directory() -> str | None`: Get current working directory (OSC 7)
- `accept_osc7() -> bool`: Check if OSC 7 (CWD) is accepted
- `set_accept_osc7(accept: bool)`: Set whether to accept OSC 7 sequences
- `shell_integration_state() -> ShellIntegration`: Get shell integration state
- `record_cwd_change(new_cwd: str, hostname: str | None = None, username: str | None = None)`: Manually record a CWD change (updates history + session variables)
- `disable_insecure_sequences() -> bool`: Check if insecure sequences are disabled
- `set_disable_insecure_sequences(disable: bool)`: Disable insecure/dangerous sequences
- `answerback_string() -> str | None`: Get the configured ENQ answerback payload (None if disabled)
- `set_answerback_string(answerback: str | None)`: Configure ENQ answerback string (None disables; default)

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

**Device Queries:**
- **Primary DA** (`CSI c` / `CSI 0 c`): Responds with `CSI ? {id} ; 1 ; 4 ; 6 ; 9 ; 15 ; 22 ; 52 c` where `{id}` is the conformance level identifier (1=VT100, 62=VT220, 63=VT320, 64=VT420, 65=VT520). Parameter 52 advertises OSC 52 clipboard support.
- **Secondary DA** (`CSI > c` / `CSI > 0 c`): Responds with `CSI > 82 ; 10000 ; 0 c` (82 = 'P' for par-term-emu).
- **XTVERSION** (`CSI > q`): Responds with `DCS > | par-term(version) ST` where `version` is the library version string.

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
- `export_graphics_json() -> str`: Export all graphics metadata as JSON for session persistence (includes placements, scrollback, animations with base64-encoded pixel data)
- `import_graphics_json(json: str) -> int`: Import graphics from JSON string (clears existing graphics first, returns count restored)
- `graphics_store() -> GraphicsStore`: Get immutable access to graphics store (Rust API only)
- `graphics_store_mut() -> GraphicsStore`: Get mutable access to graphics store (Rust API only)

**Supported Protocols:**
- **Sixel** (DCS): VT340 bitmap graphics via `DCS Pq ... ST`
- **iTerm2** (OSC 1337): Inline images via `OSC 1337 ; File=... ST`
- **Kitty** (APC G): Advanced graphics protocol with image reuse, animation, zlib compression (`o=z`), and Unicode placeholders

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
- `export_scrollback() -> str`: Export scrollback buffer as plain text

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

### Advanced Search and Regex

- `regex_search(pattern: str, case_sensitive: bool = True) -> list[RegexMatch]`: Search terminal content using regex pattern
- `get_regex_matches() -> list[RegexMatch]`: Get current regex matches
- `clear_regex_matches()`: Clear regex match highlighting
- `next_regex_match()`: Move to next regex match
- `prev_regex_match()`: Move to previous regex match
- `get_current_regex_pattern() -> str | None`: Get active regex pattern

### Mouse Tracking and Events

- `mouse_encoding() -> MouseEncoding`: Get current mouse encoding mode
- `set_mouse_encoding(encoding: MouseEncoding)`: Set mouse encoding (Default, UTF8, SGR, URXVT)
- `get_mouse_events() -> list[MouseEvent]`: Get recorded mouse events
- `get_mouse_positions() -> list[MousePosition]`: Get mouse position history
- `get_last_mouse_position() -> MousePosition | None`: Get most recent mouse position
- `clear_mouse_history()`: Clear mouse event history
- `set_max_mouse_history(max: int)`: Set maximum mouse events to track
- `record_mouse_event(event: MouseEvent)`: Record a mouse event

### Bookmarks

- `add_bookmark(row: int, label: str | None = None)`: Add bookmark at row with optional label
- `remove_bookmark(row: int)`: Remove bookmark at row
- `get_bookmarks() -> list[Bookmark]`: Get all bookmarks
- `clear_bookmarks()`: Remove all bookmarks

### Triggers & Automation

Register regex patterns to automatically match terminal output and execute actions.

#### Trigger Management

- `add_trigger(name: str, pattern: str, actions: list[TriggerAction]) -> int`: Register a trigger with a regex pattern and actions. Returns trigger ID. Raises `ValueError` for invalid regex or action types.
- `remove_trigger(trigger_id: int) -> bool`: Remove a trigger by ID. Returns `True` if found and removed.
- `set_trigger_enabled(trigger_id: int, enabled: bool) -> bool`: Enable or disable a trigger. Returns `True` if trigger exists.
- `list_triggers() -> list[Trigger]`: List all registered triggers.
- `get_trigger(trigger_id: int) -> Trigger | None`: Get trigger by ID, or `None` if not found.

#### Trigger Scanning & Matches

- `process_trigger_scans()`: Scan dirty rows for trigger matches. Called automatically in PTY mode; call manually for non-PTY terminals.
- `poll_trigger_matches() -> list[TriggerMatch]`: Get and clear pending trigger matches.
- `poll_action_results() -> list[dict]`: Get and clear pending frontend action results (Notify, MarkLine, RunCommand, PlaySound, SendText).

#### Trigger Highlights

- `get_trigger_highlights() -> list[tuple]`: Get active highlight overlays as `(row, col_start, col_end, fg, bg)` tuples. Expired highlights are filtered.
- `clear_trigger_highlights()`: Remove all trigger highlight overlays.

#### Action Types

Actions are created using `TriggerAction(action_type, params)`:

| Action Type | Parameters | Description |
|-------------|-----------|-------------|
| `"highlight"` | `bg_r`, `bg_g`, `bg_b`, `fg_r`, `fg_g`, `fg_b`, `duration_ms` | Highlight matched text with colors |
| `"notify"` | `title`, `message` | Emit notification event for frontend (supports `$1`, `$2` capture substitution) |
| `"mark_line"` | `label`, `color` (r,g,b string) | Emit mark event for frontend (with optional color) |
| `"set_variable"` | `name`, `value` | Set session variable (supports capture substitution) |
| `"run_command"` | `command`, `args` (comma-separated) | Emit command event for frontend |
| `"play_sound"` | `sound_id`, `volume` | Emit sound event for frontend |
| `"send_text"` | `text`, `delay_ms` | Emit text input event for frontend |
| `"stop"` | *(none)* | Stop processing remaining actions |

### Shell Integration Extended

Extended shell integration features beyond basic OSC 133:

- `get_command_history() -> list[CommandExecution]`: Get command execution history
- `clear_command_history()`: Clear command history
- `set_max_command_history(max: int)`: Set command history limit
- `start_command_execution(command: str)`: Mark start of command execution
- `end_command_execution(exit_code: int)`: Mark end of command with exit code (also captures the output zone's row range if an Output zone exists)
- `get_current_command() -> CommandExecution | None`: Get currently executing command
- `get_command_output(index: int) -> str | None`: Extract output text for a completed command by index (0 = most recent). Returns `None` if index is out of bounds or output has been evicted from scrollback
- `get_command_outputs() -> list[dict]`: Get all commands with extractable output text. Returns list of dicts with keys `command`, `cwd`, `exit_code`, `output`. Commands whose output has been evicted from scrollback are excluded
- `get_shell_integration_stats() -> ShellIntegrationStats`: Get shell integration statistics
- `get_cwd_changes() -> list[CwdChange]`: Get working directory change history (includes hostname/username)
- `clear_cwd_history()`: Clear CWD history
- `set_max_cwd_history(max: int)`: Set CWD history limit
- `record_cwd_change(cwd: str, hostname: str | None = None, username: str | None = None)`: Record working directory change
- `poll_events()`: Now also returns `cwd_changed` events with `old_cwd`, `new_cwd`, `hostname`, `username`, `timestamp`
- `poll_events()`: Now also returns `user_var_changed` events with `name`, `value`, `old_value` (optional) when OSC 1337 SetUserVar sequences are received
- `poll_shell_integration_events() -> list[dict]`: Drain only shell integration events (keeping other events queued). Returns dicts with `event_type`, `command`, `exit_code`, `timestamp`, `cursor_line`. The `cursor_line` is the absolute cursor line (`scrollback_len + cursor_row`) captured at the exact moment each OSC 133 marker was parsed
- `poll_events()` and `poll_subscribed_events()`: Shell integration events now include `cursor_line` field

### Semantic Zones

Semantic buffer zoning powered by OSC 133 FinalTerm shell integration markers. Zones partition the scrollback buffer into typed regions (prompt, command, output) that can be queried individually.

#### Zone Query Methods

- `get_zones() -> list[dict]`: Returns all semantic zones as a list of dictionaries. Each dict contains:
  - `zone_type` (str): `"prompt"`, `"command"`, or `"output"`
  - `abs_row_start` (int): Absolute row where zone starts
  - `abs_row_end` (int): Absolute row where zone ends (inclusive)
  - `command` (str | None): Command text (present for command and output zones)
  - `exit_code` (int | None): Exit code (present for output zones after command finishes)
  - `timestamp` (int | None): Unix milliseconds when zone was created

- `get_zone_at(abs_row: int) -> dict | None`: Returns the zone containing the given absolute row, or `None` if no zone covers that row. The returned dict has the same fields as `get_zones()`.

- `get_zone_text(abs_row: int) -> str | None`: Extracts text content from the zone containing the given absolute row. Returns `None` if no zone covers that row. Text is extracted from the grid rows spanned by the zone.

**Notes:**
- Zones are only created on the primary screen buffer; alternate screen (e.g., vim, less) does not generate zones.
- Zones are automatically evicted when their rows scroll out of the scrollback buffer.
- Zones are cleared on terminal reset.

**Example:**
```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

# Simulate a shell integration cycle:
# OSC 133;A (prompt start), OSC 133;B (command start),
# OSC 133;C (command executed), OSC 133;D;0 (command finished, exit code 0)
term.process_str("\x1b]133;A\x07")
term.process_str("$ ")
term.process_str("\x1b]133;B\x07")
term.process_str("ls -la")
term.process_str("\x1b]133;C\x07")
term.process_str("file1.txt\nfile2.txt\n")
term.process_str("\x1b]133;D;0\x07")

zones = term.get_zones()
# [
#   {"zone_type": "prompt", "abs_row_start": 0, "abs_row_end": 0, ...},
#   {"zone_type": "command", "abs_row_start": 0, "abs_row_end": 0, ...},
#   {"zone_type": "output", "abs_row_start": 0, "abs_row_end": 1, ...},
# ]

# Query a specific row
zone = term.get_zone_at(0)  # Returns the zone covering row 0

# Extract text from a zone
text = term.get_zone_text(1)  # Text from the output zone
```

### Semantic Snapshot

Structured terminal state extraction for AI/LLM consumption and external tooling. Returns a point-in-time view of terminal content, zones, commands, and metadata.

#### `get_semantic_snapshot(scope="visible", max_commands=10) -> dict`

Returns a structured snapshot as a Python dict.

**Args:**
- `scope` (`str`): Controls how much history is included:
  - `"visible"`: Only the visible screen (no scrollback, no command history)
  - `"recent"`: Last N commands with output + visible screen
  - `"full"`: Entire scrollback + all command/zone history
- `max_commands` (`int`): For `"recent"` scope, max commands to include (default: 10)

**Returns:** dict with keys:
- `timestamp` (`int`): Unix epoch milliseconds when snapshot was taken
- `cols`, `rows` (`int`): Terminal dimensions
- `title` (`str`): Terminal title (from OSC 0/2)
- `cursor_col`, `cursor_row` (`int`): Cursor position (0-indexed)
- `alt_screen_active` (`bool`): Whether alternate screen buffer is active
- `visible_text` (`str`): Plain text of visible screen
- `scrollback_text` (`str | None`): Scrollback text (Recent/Full scopes only)
- `zones` (`list[dict]`): Semantic zones with `id`, `zone_type`, `abs_row_start`, `abs_row_end`, `text`, `command`, `exit_code`, `timestamp`
- `commands` (`list[dict]`): Command history with `command`, `cwd`, `start_time`, `end_time`, `exit_code`, `duration_ms`, `success`, `output`
- `cwd`, `hostname`, `username` (`str | None`): Current environment context
- `cwd_history` (`list[dict]`): CWD change records
- `scrollback_lines`, `total_zones`, `total_commands` (`int`): Summary counts

#### `get_semantic_snapshot_json(scope="visible", max_commands=10) -> str`

Returns the same snapshot data as a JSON string. More efficient when forwarding data as a string (e.g., to an LLM API).

**Example:**
```python
term = Terminal(80, 24)
term.process(b"Hello, World!\r\n")

# Get as Python dict
snap = term.get_semantic_snapshot(scope="visible")
print(snap["cols"])  # 80
print(snap["visible_text"])  # Contains "Hello, World!"

# Get as JSON string (more efficient for API forwarding)
json_str = term.get_semantic_snapshot_json(scope="recent", max_commands=5)

# Full snapshot with all history
full = term.get_semantic_snapshot(scope="full")
for cmd in full.get("commands", []):
    print(f"{cmd['command']} -> exit {cmd.get('exit_code')}")
```

### Clipboard Extended

Advanced clipboard features beyond basic OSC 52:

- `get_clipboard_sync_events() -> list[ClipboardSyncEvent]`: Get clipboard synchronization events
- `clear_clipboard_sync_events()`: Clear clipboard sync event log
- `set_max_clipboard_sync_history(max: int)`: Set clipboard sync history limit
- `get_clipboard_sync_history() -> list[ClipboardHistoryEntry]`: Get clipboard sync history
- `record_clipboard_sync(slot: str, content: str)`: Record clipboard synchronization

### Graphics Extended

Additional graphics management beyond basic display:

- `add_inline_image(image: InlineImage)`: Add inline image (iTerm2 protocol)
- `get_image_by_id(id: int) -> InlineImage | None`: Get image by ID
- `get_images_at(row: int) -> list[InlineImage]`: Get images at specific row
- `get_all_images() -> list[InlineImage]`: Get all images in terminal
- `delete_image(id: int)`: Delete image by ID
- `clear_images()`: Clear all inline images
- `set_max_inline_images(max: int)`: Set maximum inline image count
- `get_sixel_limits() -> tuple[int, int]`: Get Sixel size limits (width, height)
- `set_sixel_limits(max_width: int, max_height: int)`: Set Sixel size limits
- `get_sixel_graphics_limit() -> int`: Get maximum Sixel graphics count
- `set_sixel_graphics_limit(limit: int)`: Set maximum Sixel graphics count
- `get_sixel_stats() -> dict[str, int]`: Get Sixel statistics
- `get_dropped_sixel_graphics() -> int`: Get count of dropped Sixel graphics

### File Transfer

General-purpose file transfer support via OSC 1337 `File=` protocol with `inline=0`. Supports both host-to-terminal downloads and terminal-to-host uploads (`RequestUpload`).

#### Query Methods

- `get_active_transfers() -> list[dict]`: Get all currently active (in-progress) file transfers. Each dict contains:
  - `id` (`int`): Unique transfer identifier
  - `direction` (`str`): `"download"` or `"upload"`
  - `filename` (`str | None`): Original filename if provided
  - `status` (`str`): Current status (`"pending"`, `"in_progress"`, `"completed"`, `"failed"`, `"cancelled"`)
  - `bytes_transferred` (`int`): Bytes received so far
  - `total_bytes` (`int | None`): Expected total size if known
- `get_completed_transfers() -> list[dict]`: Get completed transfers (without data bytes). Same dict keys as `get_active_transfers()`.
- `get_transfer(transfer_id: int) -> dict | None`: Get a specific transfer by ID. Returns `None` if not found.

#### Retrieve and Consume

- `take_completed_transfer(transfer_id: int) -> dict | None`: Remove a completed transfer from the buffer and return it with the raw file data. Returns `None` if not found. The returned dict includes all keys from `get_active_transfers()` plus:
  - `data` (`bytes`): The raw decoded file content

#### Control Methods

- `cancel_file_transfer(transfer_id: int) -> bool`: Cancel an active transfer. Returns `True` if the transfer was found and cancelled.
- `send_upload_data(data: bytes) -> None`: Send file data in response to an `upload_requested` event. Writes `ok\n` followed by base64-encoded data to the PTY.
- `cancel_upload() -> None`: Cancel a pending upload request. Writes abort sequence to the PTY.

#### Configuration

- `set_max_transfer_size(max_bytes: int) -> None`: Set the maximum allowed file transfer size in bytes. Transfers exceeding this limit will fail with a `file_transfer_failed` event.
- `get_max_transfer_size() -> int`: Get the current maximum transfer size limit.

#### Observer Events

File transfer events are delivered through the [Observer API](#observer-api). Subscribe using `kinds` filter or receive all events.

| Event Type | Dict Keys | Description |
|-----------|-----------|-------------|
| `file_transfer_started` | `id`, `direction`, `filename`, `total_bytes` | A file download or upload has begun |
| `file_transfer_progress` | `id`, `bytes_transferred`, `total_bytes` | Progress update during multipart transfer |
| `file_transfer_completed` | `id`, `filename`, `size` | Transfer finished successfully |
| `file_transfer_failed` | `id`, `reason` | Transfer failed (decode error, size exceeded, cancelled) |
| `upload_requested` | `format` | Host requested a file upload (e.g., `"tgz"`) |

#### Download Example

```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

def on_transfer(event: dict) -> None:
    if event["type"] == "file_transfer_completed":
        transfer = term.take_completed_transfer(event["id"])
        if transfer:
            with open(transfer["filename"] or "download.bin", "wb") as f:
                f.write(transfer["data"])
            print(f"Saved {transfer['filename']} ({len(transfer['data'])} bytes)")

obs_id = term.add_observer(on_transfer, kinds=[
    "file_transfer_started",
    "file_transfer_progress",
    "file_transfer_completed",
    "file_transfer_failed",
])

# Process incoming OSC 1337 File= data with inline=0
# term.process(file_transfer_data)

term.remove_observer(obs_id)
```

#### Upload Example

```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

def on_upload_request(event: dict) -> None:
    if event["type"] == "upload_requested":
        # Read file and send data
        with open("archive.tgz", "rb") as f:
            term.send_upload_data(f.read())
        # Or cancel: term.cancel_upload()

obs_id = term.add_observer(on_upload_request, kinds=["upload_requested"])
```

### Rendering and Damage Tracking

For optimized rendering in frontends:

- `add_damage_region(x: int, y: int, width: int, height: int)`: Mark region as damaged/needing redraw
- `get_damage_regions() -> list[DamageRegion]`: Get all damaged regions
- `clear_damage_regions()`: Clear damage tracking
- `merge_damage_regions()`: Merge overlapping damage regions
- `get_dirty_rows() -> list[int]`: Get rows that need redrawing
- `get_dirty_region() -> DamageRegion | None`: Get bounding box of all dirty regions
- `mark_row_dirty(row: int)`: Mark specific row as dirty
- `mark_clean()`: Mark all content as clean
- `add_rendering_hint(hint: RenderingHint)`: Add rendering optimization hint
- `get_rendering_hints() -> list[RenderingHint]`: Get rendering hints
- `clear_rendering_hints()`: Clear rendering hints

### Performance and Benchmarking

Performance measurement and optimization tools:

- `benchmark_rendering(duration_ms: int) -> BenchmarkResult`: Benchmark rendering performance
- `benchmark_parsing(duration_ms: int) -> BenchmarkResult`: Benchmark ANSI parsing performance
- `benchmark_grid_ops(iterations: int) -> BenchmarkResult`: Benchmark grid operations
- `run_benchmark_suite() -> BenchmarkSuite`: Run comprehensive benchmark suite
- `enable_profiling()`: Enable performance profiling
- `disable_profiling()`: Disable performance profiling
- `is_profiling_enabled() -> bool`: Check if profiling is active
- `get_profiling_data() -> ProfilingData`: Get profiling data
- `reset_profiling_data()`: Reset profiling counters
- `get_performance_metrics() -> PerformanceMetrics`: Get performance metrics
- `reset_performance_metrics()`: Reset performance metrics
- `get_frame_timings() -> list[FrameTiming]`: Get frame timing history
- `get_fps() -> float`: Get current FPS
- `get_average_frame_time() -> float`: Get average frame time in milliseconds
- `record_frame_timing(render_time_ms: float)`: Record frame timing

### Tmux Control Mode

Terminal multiplexer integration:

- `set_tmux_control_mode(enabled: bool)`: Enable/disable tmux control mode parsing (also enables auto-detect)
- `is_tmux_control_mode() -> bool`: Check if tmux control mode is active
- `set_tmux_auto_detect(enabled: bool)`: Enable/disable auto-detection of tmux control mode (auto-switches when `%begin` is seen)
- `is_tmux_auto_detect() -> bool`: Check if auto-detection is enabled
- `drain_tmux_notifications() -> list[TmuxNotification]`: Get and clear tmux notifications
- `get_tmux_notifications() -> list[TmuxNotification]`: Get tmux notifications without clearing
- `has_tmux_notifications() -> bool`: Check if tmux notifications are pending
- `clear_tmux_notifications()`: Clear tmux notification queue

### Session Management

Save and restore terminal state:

- `set_remote_session_id(id: str | None)`: Set remote session identifier
- `remote_session_id() -> str | None`: Get remote session identifier
- `serialize_session() -> bytes`: Serialize terminal state to bytes
- `deserialize_session(data: bytes)`: Restore terminal state from bytes
- `create_session_state() -> SessionState`: Create session state snapshot
- `capture_pane_state() -> PaneState`: Capture pane state for window management
- `restore_pane_state(state: PaneState)`: Restore pane state
- `get_pane_state() -> PaneState | None`: Get current pane state
- `set_pane_state(state: PaneState)`: Set pane state
- `clear_pane_state()`: Clear pane state
- `create_window_layout() -> WindowLayout`: Create window layout descriptor

### Advanced Text Operations

Extended text manipulation beyond basic extraction:

- `get_paragraph_at(col: int, row: int) -> str | None`: Extract paragraph at position
- `get_logical_lines(start_row: int, end_row: int) -> list[JoinedLines]`: Get logical lines (respecting wrapping)
- `join_wrapped_lines(start_row: int) -> JoinedLines`: Join wrapped lines from start position
- `is_line_start(row: int) -> bool`: Check if row is start of logical line
- `get_line_context(row: int, before: int, after: int) -> list[str]`: Get lines with context

### Testing and Compliance

VT compliance testing:

- `test_compliance() -> ComplianceReport`: Run VT compliance tests
- `format_compliance_report(report: ComplianceReport) -> str`: Format compliance report for display

### Unicode Normalization

- `normalization_form() -> NormalizationForm`: Get the current Unicode normalization form (default: NFC)
- `set_normalization_form(form: NormalizationForm)`: Set the Unicode normalization form for text stored in cells

**Normalization forms:**
- `NormalizationForm.NFC` - Canonical Composition (default): composes base + combining into precomposed form
- `NormalizationForm.NFD` - Canonical Decomposition: decomposes precomposed into base + combining marks
- `NormalizationForm.NFKC` - Compatibility Composition: NFC + replaces compatibility characters
- `NormalizationForm.NFKD` - Compatibility Decomposition: NFD + replaces compatibility characters
- `NormalizationForm.Disabled` - No normalization, store text as received from PTY

### Utility Methods

- `use_alt_screen()`: Switch to alternate screen buffer (programmatic, not via escape codes)
- `use_primary_screen()`: Switch to primary screen buffer (programmatic)
- `poll_events() -> list[str]`: Poll for pending terminal events
- `set_event_subscription(kinds: list[str] | None)`: Filter which terminal events are returned by `poll_subscribed_events()` (None clears filter)
- `clear_event_subscription()`: Clear event filter (all events are returned)
- `poll_subscribed_events() -> list[dict]`: Drain events that match subscription filter
- `poll_cwd_events() -> list[dict]`: Drain only CWD change events (fields: new_cwd, old_cwd?, hostname?, username?, timestamp)
- `poll_shell_integration_events() -> list[dict]`: Drain only shell integration events (fields: event_type, command?, exit_code?, timestamp?, cursor_line?)
- `poll_upload_requests() -> list[str]`: Drain only upload request events, returning format strings from pending `UploadRequested` events

**Event types returned by `poll_events()` / `poll_subscribed_events()`:**
`bell`, `title_changed`, `size_changed`, `mode_changed`, `graphics_added`, `hyperlink_added`, `dirty_region`, `cwd_changed`, `trigger_matched`, `user_var_changed`

The `user_var_changed` event dict contains: `name`, `value`, and optionally `old_value` (when updating an existing variable).
- `update_animations()`: Update animation frames (for blinking cursor, text, etc.)
- `debug_info() -> str`: Get debug information string
- `detect_urls(text: str) -> list[DetectedItem]`: Detect URLs in text
- `detect_file_paths(text: str) -> list[DetectedItem]`: Detect file paths in text
- `detect_semantic_items(text: str) -> list[DetectedItem]`: Detect semantic items (URLs, paths, emails)
- `get_all_hyperlinks() -> list[str]`: Get all OSC 8 hyperlinks in terminal
- `generate_color_palette() -> ColorPalette`: Generate color palette from terminal colors
- `color_distance(color1: tuple[int, int, int], color2: tuple[int, int, int]) -> float`: Calculate perceptual color distance

### Debug and Snapshot Methods

- `debug_snapshot_buffer() -> str`: Get debug snapshot of buffer state
- `debug_snapshot_grid() -> str`: Get debug snapshot of grid state
- `debug_snapshot_primary() -> str`: Get debug snapshot of primary screen
- `debug_snapshot_alt() -> str`: Get debug snapshot of alternate screen
- `debug_log_snapshot()`: Log debug snapshot to console
- `diff_snapshots(snapshot1: ScreenSnapshot, snapshot2: ScreenSnapshot) -> SnapshotDiff`: Compare two snapshots

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
- `Terminal.rgb_to_hsl_color(rgb: tuple[int, int, int]) -> ColorHSL`: Convert RGB to HSL color representation
- `Terminal.rgb_to_hsv_color(rgb: tuple[int, int, int]) -> ColorHSV`: Convert RGB to HSV color representation
- `Terminal.hsl_to_rgb_color(h: int, s: int, l: int) -> tuple[int, int, int]`: Convert HSL to RGB
- `Terminal.hsv_to_rgb_color(h: int, s: int, v: int) -> tuple[int, int, int]`: Convert HSV to RGB

### Observer API

Push-based event delivery for terminal state changes. Observers receive event dicts as they occur during `process()` calls, eliminating the need to poll for events.

#### Registration Methods

- `add_observer(callback, kinds=None) -> int`: Register a synchronous observer callback. The callback receives a `dict` for each matching event. Returns a unique observer ID.
  - `callback` (`Callable[[dict], None]`): Python callable accepting a single dict argument.
  - `kinds` (`list[str] | None`): Optional list of event type strings to filter on. When `None`, all events are delivered.
- `add_async_observer(kinds=None) -> tuple[int, asyncio.Queue]`: Register an async observer backed by an `asyncio.Queue`. Events are pushed via `put_nowait()`. Returns `(observer_id, queue)`.
  - `kinds` (`list[str] | None`): Optional list of event type strings to filter on.
- `remove_observer(observer_id) -> bool`: Remove a previously registered observer. Returns `True` if the observer was found and removed.
  - `observer_id` (`int`): The ID returned by `add_observer()` or `add_async_observer()`.
- `observer_count() -> int`: Get the number of currently registered observers.

#### Event Dict Format

All observer events are delivered as Python dicts with a `"type"` key identifying the event kind. The remaining keys depend on the event type.

**Supported event types:**

`bell`, `title_changed`, `size_changed`, `mode_changed`, `graphics_added`, `hyperlink_added`, `dirty_region`, `cwd_changed`, `trigger_matched`, `user_var_changed`, `progress_bar_changed`, `badge_changed`, `shell_integration`, `zone_opened`, `zone_closed`, `zone_scrolled_out`, `environment_changed`, `remote_host_transition`, `sub_shell_detected`, `file_transfer_started`, `file_transfer_progress`, `file_transfer_completed`, `file_transfer_failed`, `upload_requested`

#### Examples

**Synchronous observer:**
```python
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)

def on_event(event: dict) -> None:
    print(f"Event: {event['type']}")

# Observe all events
obs_id = term.add_observer(on_event)

# Observe specific event types only
obs_id = term.add_observer(on_event, kinds=["bell", "title_changed"])

# Process data — observer callback fires inline
term.process(b"\x07")  # BEL triggers bell event

# Remove when done
term.remove_observer(obs_id)
```

**Async observer with asyncio.Queue:**
```python
import asyncio
from par_term_emu_core_rust import Terminal

term = Terminal(80, 24)
obs_id, queue = term.add_async_observer(kinds=["title_changed"])

term.process(b"\x1b]0;Hello\x07")  # Set title

event = queue.get_nowait()
print(event["type"])  # "title_changed"

term.remove_observer(obs_id)
```

## Observer Convenience Functions

The `par_term_emu_core_rust.observers` module provides convenience wrappers that register observers for common event patterns. All functions return an observer ID for later removal via `terminal.remove_observer()`.

These functions are also available as top-level imports from the package.

- `on_command_complete(terminal, callback) -> int`: Register callback for command completion events. Fires when a shell integration `command_finished` event is received (OSC 133;D). Internally subscribes to `shell_integration` events and filters for `command_finished`.
- `on_zone_change(terminal, callback) -> int`: Register callback for zone lifecycle events. Fires on `zone_opened`, `zone_closed`, and `zone_scrolled_out` events.
- `on_cwd_change(terminal, callback) -> int`: Register callback for working directory changes. Fires when OSC 7 updates the current working directory (`cwd_changed` events).
- `on_title_change(terminal, callback) -> int`: Register callback for terminal title changes. Fires when OSC 0/2 updates the terminal title (`title_changed` events).
- `on_bell(terminal, callback) -> int`: Register callback for bell events. Fires when BEL (0x07) or other bell sequences are processed (`bell` events).

**Example:**
```python
from par_term_emu_core_rust import Terminal, on_bell, on_title_change

term = Terminal(80, 24)

bell_id = on_bell(term, lambda event: print("Bell!"))
title_id = on_title_change(term, lambda event: print(f"Title: {event.get('title')}"))

term.process(b"\x07")               # Prints: Bell!
term.process(b"\x1b]0;My Title\x07")  # Prints: Title: My Title

term.remove_observer(bell_id)
term.remove_observer(title_id)
```

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
- `faint_text_alpha() -> float`: Get alpha multiplier for SGR 2 (dim/faint) text (0.0-1.0, default 0.5)
- `set_faint_text_alpha(alpha: float)`: Set alpha multiplier for dim text (clamped to 0.0-1.0)

**Note:** PtyTerminal inherits all Terminal methods, so you can also use all Terminal appearance settings like `set_default_fg()`, `set_default_bg()`, etc.

#### Macro Playback (PTY-Specific)

Automate terminal interactions with recorded macros:

- `play_macro(name: str, speed: float | None = None)`: Start playing a macro (speed multiplier: 1.0 = normal, 2.0 = double)
- `stop_macro()`: Stop macro playback
- `pause_macro()`: Pause macro playback
- `resume_macro()`: Resume paused macro
- `set_macro_speed(speed: float)`: Set playback speed (0.1 to 10.0)
- `is_macro_playing() -> bool`: Check if macro is currently playing
- `is_macro_paused() -> bool`: Check if playback is paused
- `get_macro_progress() -> tuple[int, int] | None`: Get progress as (current_event, total_events)
- `get_current_macro_name() -> str | None`: Get name of playing macro
- `tick_macro() -> bool`: Advance macro playback (call regularly, e.g., every 10ms). Returns True if event was processed
- `get_macro_screenshot_triggers() -> list[str]`: Get and clear screenshot trigger labels
- `recording_to_macro(session: RecordingSession, name: str) -> Macro`: Convert recording session to macro
- `get_macro(name: str) -> Macro | None`: Get macro by name
- `list_macros() -> list[str]`: List all available macros
- `load_macro(yaml_str: str) -> Macro`: Load macro from YAML string
- `remove_macro(name: str)`: Remove macro by name

#### Coprocess Management

Run external processes alongside the terminal session, optionally feeding terminal output to their stdin.

- `start_coprocess(config: CoprocessConfig) -> int`: Start a coprocess and return its ID.
- `stop_coprocess(coprocess_id: int)`: Stop a coprocess by ID. Raises `ValueError` if not found.
- `write_to_coprocess(coprocess_id: int, data: bytes)`: Write data to coprocess stdin. Raises `ValueError` if not found.
- `read_from_coprocess(coprocess_id: int) -> list[str]`: Read buffered output lines from coprocess. Raises `ValueError` if not found.
- `list_coprocesses() -> list[int]`: List active coprocess IDs.
- `coprocess_status(coprocess_id: int) -> bool | None`: Check if coprocess is running. Returns `None` if not found.
- `read_coprocess_errors(coprocess_id: int) -> list[str]`: Read buffered stderr lines from coprocess (drains the buffer). Raises `ValueError` if not found.

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

Shell integration state (OSC 133, OSC 7, and OSC 1337 RemoteHost).

**Properties:**
- `in_prompt: bool`: True if currently in prompt (marker A)
- `in_command_input: bool`: True if currently in command input (marker B)
- `in_command_output: bool`: True if currently in command output (marker C)
- `current_command: str | None`: The command that was executed
- `last_exit_code: int | None`: Exit code from last command (marker D)
- `cwd: str | None`: Current working directory from OSC 7
- `hostname: str | None`: Remote hostname from OSC 7 or OSC 1337 RemoteHost (None for localhost)
- `username: str | None`: Username from OSC 7 or OSC 1337 RemoteHost

### Graphic

Protocol-agnostic graphic representation (Sixel, iTerm2, or Kitty).

**Properties:**
- `id: int`: Unique placement ID
- `protocol: str`: Graphics protocol used (`"sixel"`, `"iterm"`, or `"kitty"`)
- `position: tuple[int, int]`: Position in terminal `(col, row)`
- `width: int`: Width in pixels (may change during animation)
- `height: int`: Height in pixels (may change during animation)
- `original_width: int`: Original width in pixels as decoded from source image (immutable, for aspect ratio preservation)
- `original_height: int`: Original height in pixels as decoded from source image (immutable, for aspect ratio preservation)
- `scroll_offset_rows: int`: Rows scrolled off visible area (for partial rendering)
- `cell_dimensions: tuple[int, int] | None`: Cell dimensions `(cell_width, cell_height)` for rendering
- `was_compressed: bool`: Whether the original data was compressed (e.g., Kitty `o=z` zlib). Useful for diagnostics/logging.
- `placement: ImagePlacement`: Unified placement metadata (display mode, sizing, z-index, offsets)

**Methods:**
- `get_pixel(x: int, y: int) -> tuple[int, int, int, int] | None`: Get RGBA color at pixel coordinates, or `None` if out of bounds
- `pixels() -> bytes`: Get raw RGBA pixel data in row-major order
- `cell_size(cell_width: int, cell_height: int) -> tuple[int, int]`: Get size in terminal cells `(cols, rows)`
- `sample_half_block(cell_col: int, cell_row: int, cell_width: int, cell_height: int) -> tuple[tuple[int, int, int, int], tuple[int, int, int, int]] | None`: Sample top/bottom half-block colors for rendering

### ImagePlacement

Unified image placement metadata across graphics protocols. Abstracts placement info from Kitty and iTerm2 so frontends can implement inline/cover/contain rendering without protocol-specific logic.

**Properties:**
- `display_mode: str`: Display mode (`"inline"` or `"download"`)
- `requested_width: ImageDimension`: Requested width for sizing
- `requested_height: ImageDimension`: Requested height for sizing
- `preserve_aspect_ratio: bool`: Whether to preserve aspect ratio when scaling (default `True`)
- `columns: int | None`: Number of columns to display (Kitty `c=` parameter)
- `rows: int | None`: Number of rows to display (Kitty `r=` parameter)
- `z_index: int`: Z-index for layering (Kitty `z=` parameter, 0 = default)
- `x_offset: int`: X offset within the cell in pixels (Kitty `x=` parameter)
- `y_offset: int`: Y offset within the cell in pixels (Kitty `y=` parameter)

### ImageDimension

Image dimension with unit for sizing.

**Properties:**
- `value: float`: Numeric value (0 means auto)
- `unit: str`: Unit: `"auto"`, `"cells"`, `"pixels"`, or `"percent"`

**Methods:**
- `is_auto() -> bool`: Check if this is an auto dimension

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

### BenchmarkResult

Result from a single benchmark test.

**Properties:**
- `name: str`: Benchmark name
- `duration_ms: int`: Test duration in milliseconds
- `iterations: int`: Number of iterations performed
- `ops_per_sec: float`: Operations per second
- `avg_time_us: float`: Average time per operation in microseconds

### BenchmarkSuite

Results from comprehensive benchmark suite.

**Properties:**
- `rendering: BenchmarkResult`: Rendering benchmark results
- `parsing: BenchmarkResult`: Parsing benchmark results
- `grid_ops: BenchmarkResult`: Grid operations benchmark results
- `total_duration_ms: int`: Total suite duration in milliseconds

### ComplianceTest

Individual VT compliance test result.

**Properties:**
- `name: str`: Test name
- `passed: bool`: Whether test passed
- `description: str`: Test description
- `expected: str | None`: Expected behavior
- `actual: str | None`: Actual behavior

### ComplianceReport

Complete VT compliance test report.

**Properties:**
- `total_tests: int`: Total number of tests
- `passed_tests: int`: Number of passed tests
- `failed_tests: int`: Number of failed tests
- `tests: list[ComplianceTest]`: Individual test results

### CommandExecution

Command execution record from shell integration.

**Properties:**
- `command: str`: The executed command
- `start_time: int`: Start timestamp (Unix timestamp in seconds)
- `end_time: int | None`: End timestamp if completed
- `exit_code: int | None`: Exit code if completed
- `cwd: str | None`: Working directory where command was executed
- `duration_ms: int | None`: Command duration in milliseconds
- `success: bool | None`: Whether command succeeded (exit code 0)
- `output_start_row: int | None`: Absolute start row of the command's output zone
- `output_end_row: int | None`: Absolute end row of the command's output zone

### CwdChange

Working directory change event.

**Properties:**
- `cwd: str`: New working directory path
- `old_cwd: str | None`: Previous working directory (if any)
- `hostname: str | None`: Hostname associated with the new path (None for localhost)
- `username: str | None`: Username from `user@host` portion of OSC 7 (if provided)
- `timestamp: int`: Change timestamp (Unix timestamp in milliseconds)

### DamageRegion

Screen region that needs redrawing.

**Properties:**
- `x: int`: X coordinate
- `y: int`: Y coordinate
- `width: int`: Region width
- `height: int`: Region height

### DetectedItem

Detected semantic item (URL, file path, etc.).

**Properties:**
- `item_type: str`: Type of item ("url", "file_path", "email", etc.)
- `value: str`: The detected value
- `start_col: int`: Start column
- `start_row: int`: Start row
- `end_col: int`: End column
- `end_row: int`: End row

### EscapeSequenceProfile

Profile data for escape sequence parsing.

**Properties:**
- `sequence_type: str`: Type of sequence (CSI, OSC, DCS, etc.)
- `count: int`: Number of times seen
- `total_time_us: int`: Total processing time in microseconds
- `avg_time_us: float`: Average processing time in microseconds

### FrameTiming

Frame rendering timing information.

**Properties:**
- `timestamp: int`: Frame timestamp in milliseconds
- `render_time_ms: float`: Rendering time in milliseconds
- `frame_number: int`: Frame sequence number

### ImageProtocol

Graphics protocol enumeration.

**Values:**
- `ImageProtocol.Sixel`: Sixel graphics (DCS)
- `ImageProtocol.ITerm2`: iTerm2 inline images (OSC 1337)
- `ImageProtocol.Kitty`: Kitty graphics protocol (APC G)

### ImageFormat

Image format enumeration.

**Values:**
- `ImageFormat.PNG`: PNG format
- `ImageFormat.JPEG`: JPEG format
- `ImageFormat.GIF`: GIF format
- `ImageFormat.BMP`: BMP format

### InlineImage

Inline image metadata.

**Properties:**
- `id: int`: Image identifier
- `protocol: ImageProtocol`: Graphics protocol used
- `format: ImageFormat`: Image format
- `width: int`: Width in pixels
- `height: int`: Height in pixels
- `row: int`: Display row
- `col: int`: Display column
- `data: bytes`: Image data

### JoinedLines

Logical line formed by joining wrapped lines.

**Properties:**
- `text: str`: Combined text content
- `start_row: int`: Starting row
- `end_row: int`: Ending row
- `line_count: int`: Number of physical lines

### LineDiff

Difference between two lines.

**Properties:**
- `row: int`: Row number
- `old_text: str`: Old line content
- `new_text: str`: New line content
- `changed: bool`: Whether line changed

### MouseEncoding

Mouse encoding mode enumeration.

**Values:**
- `MouseEncoding.Default`: Default encoding (single byte)
- `MouseEncoding.UTF8`: UTF-8 encoding
- `MouseEncoding.SGR`: SGR 1006 encoding
- `MouseEncoding.URXVT`: URXVT encoding

### MouseEvent

Mouse event record.

**Properties:**
- `button: int`: Mouse button (0=left, 1=middle, 2=right, 64=wheel_up, 65=wheel_down)
- `col: int`: Column position
- `row: int`: Row position
- `modifiers: int`: Modifier keys bitmask
- `event_type: str`: Event type ("press", "release", "motion")
- `timestamp: int`: Event timestamp in milliseconds

### MousePosition

Mouse cursor position.

**Properties:**
- `col: int`: Column position
- `row: int`: Row position
- `timestamp: int`: Position timestamp in milliseconds

### PaneState

Terminal pane state for window management.

**Properties:**
- `content: str`: Pane content
- `cursor_col: int`: Cursor column
- `cursor_row: int`: Cursor row
- `scrollback_lines: int`: Number of scrollback lines
- `title: str`: Pane title

### PerformanceMetrics

Performance metrics collection.

**Properties:**
- `total_frames: int`: Total frames rendered
- `dropped_frames: int`: Frames dropped
- `avg_frame_time_ms: float`: Average frame time
- `peak_memory_bytes: int`: Peak memory usage
- `total_bytes_processed: int`: Total bytes processed

### ProfilingData

Performance profiling data.

**Properties:**
- `escape_sequences: list[EscapeSequenceProfile]`: Escape sequence profiles
- `total_sequences: int`: Total sequences processed
- `total_time_us: int`: Total processing time in microseconds
- `memory_allocations: int`: Number of memory allocations
- `peak_memory_bytes: int`: Peak memory usage

### RegexMatch

Regular expression match result.

**Properties:**
- `start_col: int`: Match start column
- `start_row: int`: Match start row
- `end_col: int`: Match end column
- `end_row: int`: Match end row
- `text: str`: Matched text

### RenderingHint

Rendering optimization hint.

**Properties:**
- `hint_type: str`: Hint type ("dirty_region", "cursor_moved", "scroll", etc.)
- `data: dict[str, Any]`: Hint-specific data

### SessionState

Complete terminal session state.

**Properties:**
- `session_id: str`: Session identifier
- `content: str`: Terminal content
- `scrollback: list[str]`: Scrollback buffer
- `cursor_position: tuple[int, int]`: Cursor position
- `title: str`: Terminal title
- `environment: dict[str, str]`: Environment variables

### ShellIntegrationStats

Shell integration statistics.

**Properties:**
- `total_commands: int`: Total commands executed
- `successful_commands: int`: Commands with exit code 0
- `failed_commands: int`: Commands with non-zero exit code
- `avg_command_duration_ms: float`: Average command duration
- `cwd_changes: int`: Number of directory changes

### SnapshotDiff

Difference between two screen snapshots.

**Properties:**
- `changed_lines: list[LineDiff]`: Lines that changed
- `cursor_moved: bool`: Whether cursor moved
- `old_cursor: tuple[int, int]`: Old cursor position
- `new_cursor: tuple[int, int]`: New cursor position

### TmuxNotification

Tmux control mode notification.

**Properties:**
- `notification_type: str`: Notification type
- `data: str`: Notification data

### Trigger

A registered trigger pattern.

**Properties:**
- `id: int`: Trigger ID
- `name: str`: Trigger name
- `pattern: str`: Regex pattern string
- `enabled: bool`: Whether the trigger is active
- `match_count: int`: Number of times this trigger has matched

### TriggerAction

Trigger action configuration. Constructed from Python with `TriggerAction(action_type, params)`.

**Properties:**
- `action_type: str`: Action type (e.g., "highlight", "notify", "mark_line", "set_variable", "run_command", "play_sound", "send_text", "stop")
- `params: dict[str, str]`: Action parameters (keys depend on action type)

### TriggerMatch

A trigger match result from scanning terminal output.

**Properties:**
- `trigger_id: int`: ID of the trigger that matched
- `row: int`: Row where the match occurred
- `col: int`: Grid column start of the match (accounts for wide and combining characters)
- `end_col: int`: Grid column end of the match (exclusive; accounts for wide and combining characters)
- `text: str`: Matched text
- `captures: list[str]`: Capture groups (index 0 = full match, 1+ = groups)
- `timestamp: int`: Match timestamp (Unix timestamp in seconds)

### NormalizationForm

Unicode normalization form for terminal text storage.

**Enum Values:**
- `NormalizationForm.Disabled` (0): No normalization
- `NormalizationForm.NFC` (1): Canonical Decomposition, followed by Canonical Composition (default)
- `NormalizationForm.NFD` (2): Canonical Decomposition
- `NormalizationForm.NFKC` (3): Compatibility Decomposition, followed by Canonical Composition
- `NormalizationForm.NFKD` (4): Compatibility Decomposition

**Methods:**
- `name() -> str`: Get human-readable name (`"none"`, `"NFC"`, `"NFD"`, `"NFKC"`, `"NFKD"`)
- `is_none() -> bool`: Check if normalization is disabled

### CoprocessConfig

Configuration for starting a coprocess.

**Constructor:** `CoprocessConfig(command, args=[], cwd=None, env={}, copy_terminal_output=True, restart_policy="never", restart_delay_ms=0)`

**Properties:**
- `command: str`: Command to run
- `args: list[str]`: Command arguments
- `cwd: str | None`: Working directory
- `env: dict[str, str]`: Environment variables
- `copy_terminal_output: bool`: Whether to pipe terminal output to coprocess stdin
- `restart_policy: str`: Restart policy - `"never"` (default), `"always"`, or `"on_failure"` (restart on non-zero exit)
- `restart_delay_ms: int`: Delay in milliseconds before restarting (default: 0 = immediate)

### WindowLayout

Window layout descriptor.

**Properties:**
- `layout_type: str`: Layout type ("horizontal", "vertical", "single")
- `panes: list[PaneState]`: Pane states
- `active_pane: int`: Active pane index

### ColorHSL

HSL color representation.

**Properties:**
- `h: int`: Hue (0-360)
- `s: int`: Saturation (0-100)
- `l: int`: Lightness (0-100)

### ColorHSV

HSV color representation.

**Properties:**
- `h: int`: Hue (0-360)
- `s: int`: Saturation (0-100)
- `v: int`: Value (0-100)

### ColorPalette

Terminal color palette.

**Properties:**
- `ansi_colors: list[tuple[int, int, int]]`: 16 ANSI colors (RGB)
- `default_fg: tuple[int, int, int]`: Default foreground color
- `default_bg: tuple[int, int, int]`: Default background color
- `cursor_color: tuple[int, int, int]`: Cursor color

### Bookmark

Terminal bookmark.

**Properties:**
- `row: int`: Bookmarked row
- `label: str | None`: Optional label
- `timestamp: int`: Creation timestamp (Unix timestamp in seconds)

### ClipboardHistoryEntry

Clipboard history entry with sync metadata.

**Properties:**
- `slot: str`: Clipboard slot name
- `content: str`: Clipboard content
- `timestamp: int`: Entry timestamp (Unix timestamp in seconds)
- `source: str`: Source of clipboard change

### ClipboardSyncEvent

Clipboard synchronization event.

**Properties:**
- `slot: str`: Clipboard slot
- `content: str`: Synced content
- `timestamp: int`: Sync timestamp (Unix timestamp in seconds)
- `direction: str`: Sync direction ("to_system", "from_system")

### SearchMatch

Text search match result (alias for RegexMatch with additional context).

**Properties:**
- `start_col: int`: Match start column
- `start_row: int`: Match start row
- `end_col: int`: Match end column
- `end_row: int`: Match end row
- `text: str`: Matched text
- `line_context: str | None`: Context line containing match

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

### ProgressState

Progress bar state (OSC 9;4).

**Values:**
- `ProgressState.Hidden`: Progress bar is hidden (state 0)
- `ProgressState.Normal`: Progress bar shows normal progress (state 1)
- `ProgressState.Error`: Progress bar shows error state (state 2)
- `ProgressState.Indeterminate`: Progress bar shows indeterminate/spinner state (state 3)
- `ProgressState.Warning`: Progress bar shows warning/paused state (state 4)

### Named Progress Bars (OSC 934)

The terminal supports multiple concurrent named progress bars via OSC 934 sequences.

**Methods:**
- `named_progress_bars() -> dict[str, dict]`: Get all named progress bars as `{id: {id, state, percent, label}}`
- `get_named_progress_bar(id: str) -> dict | None`: Get a specific bar by ID
- `set_named_progress_bar(id: str, state: str = "normal", percent: int = 0, label: str | None = None)`: Create/update a bar
- `remove_named_progress_bar(id: str) -> bool`: Remove a bar (returns True if it existed)
- `remove_all_named_progress_bars()`: Remove all bars

**Events:**
- Event type: `"progress_bar_changed"` (subscribe with `set_event_subscription(["progress_bar_changed"])`)
- Event dict keys: `action` ("set"/"remove"/"remove_all"), `id`, `state`, `percent`, `label`

## StreamingServer Class

WebSocket streaming server for broadcasting terminal state to connected clients. Available when the `streaming` feature is enabled.

### Constructor

```python
StreamingServer(pty_terminal: PtyTerminal, addr: str, config: StreamingConfig | None = None)
```

Create a streaming server bound to `addr` (e.g., `"127.0.0.1:8080"`). Automatically sets up output callback and PTY writer on the terminal.

### Server Lifecycle

- `start()`: Start the server in a background thread (non-blocking)
- `shutdown(reason: str)`: Shutdown the server and disconnect all clients
- `client_count() -> int`: Get the number of connected clients
- `max_clients() -> int`: Get the maximum allowed clients

### Event Polling

- `poll_resize() -> tuple[int, int] | None`: Poll for pending resize requests from clients

### Broadcasting Methods

Methods for sending events to all connected clients:

- `send_output(data: str)`: Send terminal output data
- `send_resize(cols: int, rows: int)`: Send terminal resize event
- `send_title(title: str)`: Send title change
- `send_bell()`: Send bell event
- `send_mode_changed(mode: str, enabled: bool)`: Send terminal mode change
- `send_graphics_added(row: int)`: Send graphics added event
- `send_hyperlink_added(url: str, row: int, col: int, id: str | None = None)`: Send hyperlink added event
- `send_user_var_changed(name: str, value: str, old_value: str | None = None)`: Send user variable change
- `send_cursor_position(col: int, row: int, visible: bool)`: Send cursor position update
- `send_badge_changed(badge: str | None = None)`: Send badge change event
- `send_action_notify(trigger_id: int, title: str, message: str)`: Send trigger notification action
- `send_action_mark_line(trigger_id: int, row: int, label: str | None = None, color: tuple[int, int, int] | None = None)`: Send trigger mark line action
- `send_cwd_changed(new_cwd: str, old_cwd: str | None = None, hostname: str | None = None, username: str | None = None, timestamp: int = 0)`: Send working directory change event
- `send_trigger_matched(trigger_id: int, row: int, col: int, end_col: int, text: str, captures: list[str] = [], timestamp: int = 0)`: Send trigger match event
- `send_progress_bar_changed(action: str, id: str, state: ProgressState | None = None, percent: int | None = None, label: str | None = None)`: Send progress bar state change. `action` must be `"set"`, `"remove"`, or `"remove_all"`.

### Theme

- `create_theme_info(name: str, background: tuple, foreground: tuple, normal: list[tuple], bright: list[tuple]) -> dict`: Create a theme info dictionary for the protocol (static method)

### Example

```python
from par_term_emu_core_rust import PtyTerminal, StreamingServer, StreamingConfig

with PtyTerminal(80, 24) as term:
    term.spawn_shell()
    config = StreamingConfig(api_key="secret", enable_system_stats=True)
    server = StreamingServer(term, "127.0.0.1:8080", config)
    server.start()

    # Broadcast events
    server.send_cwd_changed("/home/user", hostname="myhost")
    server.send_trigger_matched(1, row=5, col=0, end_col=10, text="ERROR")
    server.send_progress_bar_changed("set", "build", percent=75, label="Building...")

    server.shutdown("Done")
```

## StreamingConfig Class

Configuration for the streaming server.

### Constructor

```python
StreamingConfig(
    max_clients: int = 1000,
    send_initial_screen: bool = True,
    keepalive_interval: int = 30,
    default_read_only: bool = False,
    enable_http: bool = False,
    web_root: str = "./web_term",
    initial_cols: int = 0,
    initial_rows: int = 0,
    max_sessions: int = 10,
    session_idle_timeout: int = 900,
    max_clients_per_session: int = 0,
    input_rate_limit_bytes_per_sec: int = 0,
    api_key: str | None = None,
    enable_system_stats: bool = False,
    system_stats_interval_secs: int = 5,
)
```

### Properties (getter/setter)

- `max_clients: int` - Maximum concurrent client connections
- `send_initial_screen: bool` - Send screen snapshot on connect
- `keepalive_interval: int` - Ping interval in seconds (0=disabled)
- `default_read_only: bool` - New clients read-only by default
- `enable_http: bool` - Enable HTTP static file serving
- `web_root: str` - Web root directory for static files
- `initial_cols: int` - Initial terminal columns (0=use terminal's current size)
- `initial_rows: int` - Initial terminal rows (0=use terminal's current size)
- `max_sessions: int` - Maximum concurrent terminal sessions
- `session_idle_timeout: int` - Idle session timeout in seconds (0=never timeout)
- `max_clients_per_session: int` - Maximum clients per session (0=unlimited)
- `input_rate_limit_bytes_per_sec: int` - Input rate limit (0=unlimited)
- `api_key: str | None` - API key for authenticating API routes (masked in `__repr__`)
- `enable_system_stats: bool` - Enable system resource statistics collection
- `system_stats_interval_secs: int` - System stats collection interval in seconds
- `tls_enabled: bool` - Check if TLS is configured (read-only)

### TLS Methods

- `set_tls_from_files(cert_path: str, key_path: str)`: Configure TLS from separate cert and key files
- `set_tls_from_pem(pem_path: str)`: Configure TLS from combined PEM file
- `disable_tls()`: Disable TLS

## Streaming Functions

These functions are available when the `streaming` feature is enabled. They encode and decode protobuf messages for the streaming terminal server protocol.

### encode_server_message

```python
encode_server_message(message_type: str, **kwargs) -> bytes
```

Encode a server message into binary protobuf format.

**Supported message types:**

| Message Type | Required kwargs | Description |
|---|---|---|
| `"output"` | `data` | Terminal output data |
| `"resize"` | `cols`, `rows` | Terminal resize event |
| `"title"` | `title` | Title change |
| `"bell"` | *(none)* | Bell event |
| `"pong"` | *(none)* | Ping response |
| `"connected"` | `cols`, `rows`, `session_id`, `initial_screen`, `theme` | Connection acknowledgement |
| `"error"` | `message`, `code` | Error message |
| `"shutdown"` | `reason` | Server shutdown |
| `"cursor"` | `col`, `row`, `visible` | Cursor position update |
| `"refresh"` | `cols`, `rows`, `screen_content` | Full screen refresh |
| `"action_notify"` | `trigger_id`, `title`, `message` | Trigger notification action |
| `"action_mark_line"` | `trigger_id`, `row`, `label`, `color` | Trigger mark line action |
| `"mode_changed"` | `mode`, `enabled` | Terminal mode change |
| `"graphics_added"` | `row`, `format` | Graphics added event |
| `"hyperlink_added"` | `url`, `row`, `col`, `id` | Hyperlink added event |
| `"badge_changed"` | `badge` | Badge change event |
| `"selection_changed"` | `start_col`, `start_row`, `end_col`, `end_row`, `text`, `mode`, `cleared` | Selection change event |
| `"clipboard_sync"` | `operation`, `content`, `target` | Clipboard sync event |
| `"shell_integration"` | `event_type`, `command`, `exit_code`, `timestamp` | Shell integration event |
| `"cwd_changed"` | `new_cwd`, `old_cwd`, `hostname`, `username`, `timestamp` | Working directory changed |
| `"trigger_matched"` | `trigger_id`, `row`, `col`, `end_col`, `text`, `captures`, `timestamp` | Trigger pattern matched |
| `"user_var_changed"` | `name`, `value`, `old_value` | User variable changed (OSC 1337 SetUserVar) |
| `"progress_bar_changed"` | `action`, `id`, `state`, `percent`, `label` | Progress bar state changed (OSC 9;4 / OSC 934) |
| `"system_stats"` | *(none — server-generated)* | System resource statistics (CPU, memory, disk, network) |

**Example:**
```python
from par_term_emu_core_rust import encode_server_message

msg = encode_server_message("output", data="Hello, world!")
msg = encode_server_message("cwd_changed", new_cwd="/home/user", hostname="server1")
msg = encode_server_message("trigger_matched", trigger_id=1, row=5, col=0, end_col=10, text="error", captures=["error"], timestamp=1234567890)
msg = encode_server_message("progress_bar_changed", action="set", id="dl-1", state="normal", percent=50, label="Downloading")
```

### decode_server_message

```python
decode_server_message(data: bytes) -> dict
```

Decode a binary protobuf server message into a Python dict with a `"type"` key and message-specific fields.

**`system_stats` message fields:**

When `type` is `"system_stats"`, the dict contains:

| Field | Type | Description |
|---|---|---|
| `cpu` | `dict \| None` | `overall_usage_percent`, `physical_core_count`, `per_core_usage_percent`, `brand`, `frequency_mhz` |
| `memory` | `dict \| None` | `total_bytes`, `used_bytes`, `available_bytes`, `swap_total_bytes`, `swap_used_bytes` |
| `disks` | `list[dict]` | Each: `name`, `mount_point`, `total_bytes`, `available_bytes`, `kind`, `file_system`, `is_removable` |
| `networks` | `list[dict]` | Each: `name`, `received_bytes`, `transmitted_bytes`, `total_received_bytes`, `total_transmitted_bytes`, `packets_received`, `packets_transmitted`, `errors_received`, `errors_transmitted` |
| `load_average` | `dict \| None` | `one_minute`, `five_minutes`, `fifteen_minutes` |
| `hostname` | `str \| None` | System hostname |
| `os_name` | `str \| None` | Operating system name |
| `os_version` | `str \| None` | OS version string |
| `kernel_version` | `str \| None` | Kernel version string |
| `uptime_secs` | `int \| None` | System uptime in seconds |
| `timestamp` | `int \| None` | Unix epoch milliseconds |

## Instant Replay

Cell-level terminal snapshots with input-stream delta recording and timeline navigation. Allows reconstructing exact terminal state at any point in history.

### Snapshot Capture (Rust)

#### TerminalSnapshot

Complete capture of terminal state at a point in time. Defined in `src/terminal/terminal_snapshot.rs`.

**Fields:**
- `timestamp` (`u64`): Unix timestamp in milliseconds when the snapshot was captured
- `cols` / `rows` (`usize`): Terminal dimensions
- `grid` / `alt_grid` (`GridSnapshot`): Primary and alternate screen grid snapshots
- `alt_screen_active` (`bool`): Whether the alternate screen is active
- `cursor` / `alt_cursor` / `saved_cursor` (`Cursor`): Cursor states
- `fg` / `bg` / `underline_color`: Current drawing colors
- `flags` (`CellFlags`): Current cell attribute flags
- `saved_fg` / `saved_bg` / `saved_underline_color` / `saved_flags`: Saved drawing state
- `title` (`String`): Terminal title
- Terminal modes: `auto_wrap`, `origin_mode`, `insert_mode`, `reverse_video`, `line_feed_new_line_mode`, `application_cursor`, `bracketed_paste`, `focus_tracking`, `mouse_mode`, `mouse_encoding`, `use_lr_margins`, `left_margin`, `right_margin`, `keyboard_flags`, `modify_other_keys_mode`, `char_protected`, `bold_brightening`
- `scroll_region_top` / `scroll_region_bottom` (`usize`): Scroll region bounds
- `tab_stops` (`Vec<bool>`): Tab stop positions
- `pending_wrap` (`bool`): Delayed wrap flag
- `estimated_size_bytes` (`usize`): Approximate memory footprint

**Methods:**
- `estimate_size() -> usize`: Calculate the memory footprint of this snapshot

#### GridSnapshot

Snapshot of a single grid (primary or alternate screen). Defined in `src/terminal/terminal_snapshot.rs`.

**Fields:**
- `cells` (`Vec<Cell>`): Visible screen cells (row-major, cols * rows)
- `scrollback_cells` (`Vec<Cell>`): Scrollback buffer cells (linearized circular buffer)
- `scrollback_start` / `scrollback_lines` / `max_scrollback` (`usize`): Scrollback state
- `cols` / `rows` (`usize`): Grid dimensions
- `wrapped` / `scrollback_wrapped` (`Vec<bool>`): Line-wrap flags
- `zones` (`Vec<Zone>`): Semantic zones
- `total_lines_scrolled` (`usize`): Total lines ever scrolled into scrollback

#### Terminal Methods

- `capture_snapshot() -> TerminalSnapshot`: Capture a complete cell-level snapshot of terminal state including grids, cursors, colors, attributes, modes, scroll regions, and tab stops.
- `restore_from_snapshot(snapshot: &TerminalSnapshot)`: Restore terminal state from a previously captured snapshot. Only restores if dimensions match.

### SnapshotManager

Manages a rolling buffer of terminal snapshots with size-based eviction and input-stream recording. Defined in `src/terminal/snapshot_manager.rs`.

**Constants:**
- `DEFAULT_MAX_MEMORY_BYTES`: 4 MiB
- `DEFAULT_SNAPSHOT_INTERVAL_SECS`: 30 seconds

#### Constructor

- `new(max_memory_bytes: usize, snapshot_interval: Duration) -> SnapshotManager`
- `with_defaults() -> SnapshotManager`: Creates a manager with 4 MiB budget and 30-second interval

#### Snapshot Operations

- `take_snapshot(&mut self, terminal: &Terminal) -> usize`: Capture a snapshot and append it. Returns the new entry index. Evicts oldest entries if over budget.
- `record_input(&mut self, bytes: &[u8])`: Append input bytes to the most recent entry. No-op if disabled or empty.
- `reconstruct_at(entry_index: usize, byte_offset: usize) -> Option<Terminal>`: Reconstruct terminal state by restoring a snapshot and replaying `input_bytes[..byte_offset]`.
- `find_entry_for_timestamp(timestamp: u64) -> Option<usize>`: Binary search for the entry whose timestamp is closest to (but not after) the given Unix-millisecond timestamp.

#### Query Methods

- `entry_count() -> usize`: Number of stored entries
- `get_entry(index: usize) -> Option<&SnapshotEntry>`: Get entry at index
- `memory_usage() -> usize`: Current total memory usage
- `max_memory() -> usize`: Maximum memory budget
- `snapshot_interval() -> Duration`: Current snapshot interval
- `time_range() -> Option<(u64, u64)>`: Oldest and newest snapshot timestamps
- `should_snapshot() -> bool`: Whether enough time has elapsed for a new snapshot
- `is_enabled() -> bool`: Whether the manager is enabled

#### Configuration

- `set_max_memory(max_bytes: usize)`: Set memory budget (triggers eviction if needed)
- `set_snapshot_interval(interval: Duration)`: Set snapshot interval
- `set_enabled(enabled: bool)`: Enable or disable the manager
- `clear()`: Clear all entries and reset memory tracking

#### SnapshotEntry

A single entry in the snapshot ring buffer.

- `snapshot` (`TerminalSnapshot`): The captured terminal state
- `input_bytes` (`Vec<u8>`): Bytes fed to `Terminal::process()` after this snapshot
- `size_bytes() -> usize`: Total memory footprint of this entry

### ReplaySession

Timeline navigation for replay. Creates an independent copy of snapshot data and allows seeking through terminal history. Defined in `src/terminal/replay.rs`.

#### Constructor

- `new(manager: &SnapshotManager) -> Option<ReplaySession>`: Create a session starting at the end of the timeline. Returns `None` if the manager is empty.

#### Navigation

All navigation methods return a `SeekResult` enum (`Ok`, `AtStart`, `AtEnd`, `Empty`).

- `seek_to(entry_index: usize, byte_offset: usize) -> SeekResult`: Seek to a specific position (values are clamped to valid ranges)
- `seek_to_timestamp(timestamp: u64) -> SeekResult`: Seek to the entry closest to the given timestamp
- `seek_to_start() -> SeekResult`: Seek to the beginning of the timeline
- `seek_to_end() -> SeekResult`: Seek to the end of the timeline
- `step_forward(n_bytes: usize) -> SeekResult`: Step forward by N bytes in the input stream (crosses entry boundaries)
- `step_backward(n_bytes: usize) -> SeekResult`: Step backward by N bytes in the input stream (crosses entry boundaries)
- `next_entry() -> SeekResult`: Navigate to the start of the next entry
- `previous_entry() -> SeekResult`: Navigate to the start of the previous entry

#### Query

- `current_frame() -> &Terminal`: Reconstructed terminal at the current position
- `current_index() -> usize`: Current entry index
- `current_byte_offset() -> usize`: Byte offset within the current entry
- `total_entries() -> usize`: Total number of entries
- `current_timestamp() -> u64`: Timestamp of the current entry's snapshot

### Instant Replay Python Binding

- `capture_replay_snapshot() -> dict`: Capture a cell-level snapshot of terminal state and return metadata

**Returns:** dict with keys:
- `timestamp` (`int`): Unix timestamp in milliseconds
- `cols` (`int`): Terminal width in columns
- `rows` (`int`): Terminal height in rows
- `estimated_size_bytes` (`int`): Approximate memory footprint in bytes

```python
term = Terminal(80, 24)
term.process(b"Hello, world!")
info = term.capture_replay_snapshot()
print(f"Snapshot at {info['timestamp']}, size: {info['estimated_size_bytes']} bytes")
# Snapshot at 1707000000000, size: 47232 bytes
```

## C-Compatible FFI

The library provides a C-compatible FFI layer for embedding the terminal emulator in C/C++ applications. All types use `#[repr(C)]` for ABI stability.

### SharedCell

A frozen cell value with text, colors, and attributes.

| Field | Type | Description |
|---|---|---|
| `text` | `[u8; 4]` | UTF-8 encoded character bytes (up to 4 bytes) |
| `text_len` | `u8` | Number of valid bytes in `text` |
| `fg_r`, `fg_g`, `fg_b` | `u8` | Foreground color (resolved to RGB) |
| `bg_r`, `bg_g`, `bg_b` | `u8` | Background color (resolved to RGB) |
| `bold` | `bool` | Bold attribute |
| `italic` | `bool` | Italic attribute |
| `underline` | `bool` | Underline attribute |
| `strikethrough` | `bool` | Strikethrough attribute |

### SharedState

A frozen snapshot of the full terminal state, allocated on the heap.

| Field | Type | Description |
|---|---|---|
| `cols` | `u32` | Number of columns |
| `rows` | `u32` | Number of rows |
| `cursor_col` | `u32` | Cursor column (0-indexed) |
| `cursor_row` | `u32` | Cursor row (0-indexed) |
| `title` | `*mut c_char` | Null-terminated title string (owned) |
| `cwd` | `*mut c_char` | Null-terminated CWD string (owned) |
| `cells` | `*mut SharedCell` | Row-major cell array of `cols * rows` entries (owned) |
| `cell_count` | `u32` | Number of cells in the array |

### C API Functions

```c
// Get a frozen snapshot of the terminal state.
// Returns a heap-allocated SharedState that must be freed with terminal_free_state().
SharedState* terminal_get_state(const Terminal* term);

// Free a SharedState previously returned by terminal_get_state().
void terminal_free_state(SharedState* state);

// Register a C observer via vtable. Returns an observer ID (0 on error).
uint64_t terminal_add_observer(Terminal* term, TerminalObserverVtable vtable);

// Remove a previously registered observer by ID. Returns true if found.
bool terminal_remove_observer(Terminal* term, uint64_t id);
```

### TerminalObserverVtable

A C function-pointer table for receiving terminal events. Each callback receives a `user_data` pointer and a JSON-encoded event string (valid only for the duration of the callback).

```c
typedef struct {
    // Called for zone lifecycle events (prompt start/end, command start/end, output start/end)
    void (*on_zone_event)(void* user_data, const char* event_json);     // optional
    // Called for command completion events
    void (*on_command_event)(void* user_data, const char* event_json);  // optional
    // Called for environment change events (CWD, title, etc.)
    void (*on_environment_event)(void* user_data, const char* event_json); // optional
    // Called for screen-related events (bell, resize, graphics, file transfers, etc.)
    void (*on_screen_event)(void* user_data, const char* event_json);   // optional
    // Opaque user data passed to every callback
    void* user_data;
} TerminalObserverVtable;
```

## See Also

- [VT Sequences Reference](VT_SEQUENCES.md) - Complete list of supported ANSI/VT sequences
- [Advanced Features](ADVANCED_FEATURES.md) - Detailed feature documentation
- [Examples](../examples/) - Usage examples and demonstrations
