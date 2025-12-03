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
  - [Shell Integration Extended](#shell-integration-extended)
  - [Clipboard Extended](#clipboard-extended)
  - [Graphics Extended](#graphics-extended)
  - [Rendering and Damage Tracking](#rendering-and-damage-tracking)
  - [Performance and Benchmarking](#performance-and-benchmarking)
  - [Tmux Control Mode](#tmux-control-mode)
  - [Session Management](#session-management)
  - [Advanced Text Operations](#advanced-text-operations)
  - [Testing and Compliance](#testing-and-compliance)
  - [Utility Methods](#utility-methods)
  - [Debug and Snapshot Methods](#debug-and-snapshot-methods)
  - [Text Extraction and Selection](#text-extraction-and-selection)
  - [Content Search](#content-search)
  - [Buffer Statistics](#buffer-statistics)
  - [Static Utility Methods](#static-utility-methods)
- [PtyTerminal Class](#ptyterminal-class)
  - [Process Management](#process-management)
  - [I/O Operations](#io-operations)
  - [Update Tracking](#update-tracking)
  - [Appearance Settings](#appearance-settings-pty-specific)
  - [Macro Playback](#macro-playback-pty-specific)
  - [Context Manager Support](#context-manager-support)
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

### Shell Integration Extended

Extended shell integration features beyond basic OSC 133:

- `get_command_history() -> list[CommandExecution]`: Get command execution history
- `clear_command_history()`: Clear command history
- `set_max_command_history(max: int)`: Set command history limit
- `start_command_execution(command: str)`: Mark start of command execution
- `end_command_execution(exit_code: int)`: Mark end of command with exit code
- `get_current_command() -> CommandExecution | None`: Get currently executing command
- `get_shell_integration_stats() -> ShellIntegrationStats`: Get shell integration statistics
- `get_cwd_changes() -> list[CwdChange]`: Get working directory change history
- `clear_cwd_history()`: Clear CWD history
- `set_max_cwd_history(max: int)`: Set CWD history limit
- `record_cwd_change(cwd: str)`: Record working directory change

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

- `set_tmux_control_mode(enabled: bool)`: Enable/disable tmux control mode parsing
- `is_tmux_control_mode() -> bool`: Check if tmux control mode is active
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

### Utility Methods

- `use_alt_screen()`: Switch to alternate screen buffer (programmatic, not via escape codes)
- `use_primary_screen()`: Switch to primary screen buffer (programmatic)
- `poll_events() -> list[str]`: Poll for pending terminal events
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

### CwdChange

Working directory change event.

**Properties:**
- `cwd: str`: New working directory path
- `timestamp: int`: Change timestamp (Unix timestamp in seconds)

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
- `ProgressState.Hidden`: Progress bar is hidden
- `ProgressState.Indeterminate`: Progress bar shows indeterminate/spinner state
- `ProgressState.Normal`: Progress bar shows normal progress
- `ProgressState.Paused`: Progress bar is paused
- `ProgressState.Error`: Progress bar shows error state

## See Also

- [VT Sequences Reference](VT_SEQUENCES.md) - Complete list of supported ANSI/VT sequences
- [Advanced Features](ADVANCED_FEATURES.md) - Detailed feature documentation
- [Examples](../examples/) - Usage examples and demonstrations
