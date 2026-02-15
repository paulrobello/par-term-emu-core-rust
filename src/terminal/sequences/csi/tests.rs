use crate::color::{Color, NamedColor};
use crate::cursor::CursorStyle;
use crate::mouse::{MouseEncoding, MouseMode};
use crate::terminal::{Terminal, TerminalEvent};

// ========== Cursor Movement Tests ==========

#[test]
fn test_cursor_up() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;10H"); // Move to (10,10)

    term.process(b"\x1b[5A"); // Move up 5
    assert_eq!(term.cursor.row, 4); // 10-1-5 = 4 (0-indexed)

    term.process(b"\x1b[A"); // Default (1)
    assert_eq!(term.cursor.row, 3);

    term.process(b"\x1b[0A"); // 0 treated as 1
    assert_eq!(term.cursor.row, 2);
}

#[test]
fn test_cursor_down() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;10H"); // Move to (10,5)

    term.process(b"\x1b[3B"); // Move down 3
    assert_eq!(term.cursor.row, 7); // 5-1+3 = 7 (0-indexed)

    // Test bounds
    term.process(b"\x1b[100B");
    assert_eq!(term.cursor.row, 23); // Last row (0-indexed)
}

#[test]
fn test_cursor_forward() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;10H");

    term.process(b"\x1b[5C"); // Move right 5
    assert_eq!(term.cursor.col, 14); // 10-1+5 = 14 (0-indexed)

    // Test bounds
    term.process(b"\x1b[100C");
    assert_eq!(term.cursor.col, 79); // Last column
}

#[test]
fn test_cursor_back() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H");

    term.process(b"\x1b[10D"); // Move left 10
    assert_eq!(term.cursor.col, 9); // 20-1-10 = 9 (0-indexed)

    // Test bounds
    term.process(b"\x1b[100D");
    assert_eq!(term.cursor.col, 0); // First column
}

#[test]
fn test_cursor_position() {
    let mut term = Terminal::new(80, 24);

    // CUP - Cursor Position (1-indexed)
    term.process(b"\x1b[10;20H");
    assert_eq!(term.cursor.row, 9); // 0-indexed
    assert_eq!(term.cursor.col, 19);

    // Default position (1,1)
    term.process(b"\x1b[H");
    assert_eq!(term.cursor.row, 0);
    assert_eq!(term.cursor.col, 0);

    // HVP (same as CUP)
    term.process(b"\x1b[5;10f");
    assert_eq!(term.cursor.row, 4);
    assert_eq!(term.cursor.col, 9);
}

#[test]
fn test_cursor_horizontal_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;10H");

    // CHA - Move to column 30 (1-indexed)
    term.process(b"\x1b[30G");
    assert_eq!(term.cursor.col, 29); // 0-indexed
    assert_eq!(term.cursor.row, 9); // Row unchanged
}

#[test]
fn test_cursor_vertical_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;10H");

    // VPA - Move to row 15 (1-indexed)
    term.process(b"\x1b[15d");
    assert_eq!(term.cursor.row, 14); // 0-indexed
    assert_eq!(term.cursor.col, 9); // Column unchanged
}

#[test]
fn test_cursor_next_prev_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H");

    // CNL - Cursor next line (move down and to column 0)
    term.process(b"\x1b[3E");
    assert_eq!(term.cursor.row, 12); // 10-1+3 = 12
    assert_eq!(term.cursor.col, 0);

    // CPL - Cursor previous line (move up and to column 0)
    term.process(b"\x1b[5F");
    assert_eq!(term.cursor.row, 7); // 12-5 = 7
    assert_eq!(term.cursor.col, 0);
}

// ========== SGR Attribute Tests ==========

#[test]
fn test_sgr_reset() {
    let mut term = Terminal::new(80, 24);

    // Set some attributes
    term.process(b"\x1b[1;31;42m"); // Bold, red fg, green bg

    // Reset all
    term.process(b"\x1b[0m");
    assert!(!term.flags.bold());
}

#[test]
fn test_sgr_bold_dim_italic() {
    let mut term = Terminal::new(80, 24);

    // Bold
    term.process(b"\x1b[1m");
    assert!(term.flags.bold());

    // Dim
    term.process(b"\x1b[2m");
    assert!(term.flags.dim());

    // Italic
    term.process(b"\x1b[3m");
    assert!(term.flags.italic());

    // Reset bold/dim
    term.process(b"\x1b[22m");
    assert!(!term.flags.bold());
    assert!(!term.flags.dim());

    // Reset italic
    term.process(b"\x1b[23m");
    assert!(!term.flags.italic());
}

#[test]
fn test_sgr_underline() {
    let mut term = Terminal::new(80, 24);

    // Underline
    term.process(b"\x1b[4m");
    assert!(term.flags.underline());

    // No underline
    term.process(b"\x1b[24m");
    assert!(!term.flags.underline());
}

#[test]
fn test_sgr_other_attributes() {
    let mut term = Terminal::new(80, 24);

    // Blink
    term.process(b"\x1b[5m");
    assert!(term.flags.blink());
    term.process(b"\x1b[25m");
    assert!(!term.flags.blink());

    // Reverse
    term.process(b"\x1b[7m");
    assert!(term.flags.reverse());
    term.process(b"\x1b[27m");
    assert!(!term.flags.reverse());

    // Hidden
    term.process(b"\x1b[8m");
    assert!(term.flags.hidden());
    term.process(b"\x1b[28m");
    assert!(!term.flags.hidden());

    // Strikethrough
    term.process(b"\x1b[9m");
    assert!(term.flags.strikethrough());
    term.process(b"\x1b[29m");
    assert!(!term.flags.strikethrough());
}

#[test]
fn test_sgr_basic_colors() {
    let mut term = Terminal::new(80, 24);

    // Foreground colors (30-37)
    term.process(b"\x1b[31m"); // Red
    assert_eq!(term.fg, Color::Named(NamedColor::Red));

    term.process(b"\x1b[34m"); // Blue
    assert_eq!(term.fg, Color::Named(NamedColor::Blue));

    // Background colors (40-47)
    term.process(b"\x1b[42m"); // Green
    assert_eq!(term.bg, Color::Named(NamedColor::Green));

    // Bright colors (90-97)
    term.process(b"\x1b[91m"); // Bright red
    assert_eq!(term.fg, Color::Named(NamedColor::BrightRed));

    // Reset to defaults
    term.process(b"\x1b[39m");
    assert_eq!(term.fg, term.default_fg);
    term.process(b"\x1b[49m");
    assert_eq!(term.bg, term.default_bg);
}

#[test]
fn test_sgr_rgb_colors() {
    let mut term = Terminal::new(80, 24);

    // Foreground RGB (38;2;r;g;b)
    term.process(b"\x1b[38;2;255;128;64m");
    assert_eq!(term.fg, Color::Rgb(255, 128, 64));

    // Background RGB (48;2;r;g;b)
    term.process(b"\x1b[48;2;10;20;30m");
    assert_eq!(term.bg, Color::Rgb(10, 20, 30));
}

#[test]
fn test_sgr_256_colors() {
    let mut term = Terminal::new(80, 24);

    // Foreground 256 color (38;5;idx)
    term.process(b"\x1b[38;5;123m");
    assert_eq!(term.fg, Color::from_ansi_code(123));

    // Background 256 color (48;5;idx)
    term.process(b"\x1b[48;5;200m");
    assert_eq!(term.bg, Color::from_ansi_code(200));
}

// ========== Mode Tests ==========

#[test]
fn test_private_mode_cursor_visibility() {
    let mut term = Terminal::new(80, 24);

    // Show cursor
    term.process(b"\x1b[?25h");
    assert!(term.cursor.visible);

    // Hide cursor
    term.process(b"\x1b[?25l");
    assert!(!term.cursor.visible);
}

#[test]
fn test_private_mode_application_cursor() {
    let mut term = Terminal::new(80, 24);

    // Enable application cursor
    term.process(b"\x1b[?1h");
    assert!(term.application_cursor);

    // Disable
    term.process(b"\x1b[?1l");
    assert!(!term.application_cursor);
}

#[test]
fn test_private_mode_autowrap() {
    let mut term = Terminal::new(80, 24);

    // Disable autowrap
    term.process(b"\x1b[?7l");
    assert!(!term.auto_wrap);

    // Enable autowrap
    term.process(b"\x1b[?7h");
    assert!(term.auto_wrap);
}

#[test]
fn test_private_mode_alt_screen() {
    let mut term = Terminal::new(80, 24);

    // Switch to alternate screen
    term.process(b"\x1b[?1049h");
    assert!(term.alt_screen_active);

    // Switch back to primary
    term.process(b"\x1b[?1049l");
    assert!(!term.alt_screen_active);
}

#[test]
fn test_private_mode_mouse() {
    let mut term = Terminal::new(80, 24);

    // Normal mouse tracking
    term.process(b"\x1b[?1000h");
    assert!(matches!(term.mouse_mode, MouseMode::Normal));

    // Button event mode
    term.process(b"\x1b[?1002h");
    assert!(matches!(term.mouse_mode, MouseMode::ButtonEvent));

    // Any event mode
    term.process(b"\x1b[?1003h");
    assert!(matches!(term.mouse_mode, MouseMode::AnyEvent));

    // Disable
    term.process(b"\x1b[?1000l");
    assert!(matches!(term.mouse_mode, MouseMode::Off));
}

#[test]
fn test_private_mode_mouse_encoding() {
    let mut term = Terminal::new(80, 24);

    // SGR mouse
    term.process(b"\x1b[?1006h");
    assert!(matches!(term.mouse_encoding, MouseEncoding::Sgr));

    // UTF-8 mouse
    term.process(b"\x1b[?1005h");
    assert!(matches!(term.mouse_encoding, MouseEncoding::Utf8));

    // URXVT mouse
    term.process(b"\x1b[?1015h");
    assert!(matches!(term.mouse_encoding, MouseEncoding::Urxvt));

    // Reset to default
    term.process(b"\x1b[?1006l");
    assert!(matches!(term.mouse_encoding, MouseEncoding::Default));
}

#[test]
fn test_private_mode_bracketed_paste() {
    let mut term = Terminal::new(80, 24);

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");
    assert!(term.bracketed_paste);

    // Disable
    term.process(b"\x1b[?2004l");
    assert!(!term.bracketed_paste);
}

// ========== Device Response Tests ==========

#[test]
fn test_device_status_report() {
    let mut term = Terminal::new(80, 24);

    // DSR 5 - Operating status
    term.process(b"\x1b[5n");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[0n");

    // DSR 6 - Cursor position report
    term.process(b"\x1b[10;20H");
    term.process(b"\x1b[6n");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[10;20R"); // 1-indexed
}

#[test]
fn test_device_attributes() {
    let mut term = Terminal::new(80, 24);

    // Primary DA - should include parameter 52 for OSC 52 clipboard
    term.process(b"\x1b[c");
    let response = term.drain_responses();
    let response_str = std::str::from_utf8(&response).unwrap();
    assert!(response_str.starts_with("\x1b[?"));
    assert!(
        response_str.contains(";52"),
        "DA1 should advertise OSC 52 clipboard (param 52)"
    );

    // Secondary DA
    term.process(b"\x1b[>c");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[>82;10000;0c");
}

#[test]
fn test_xtversion() {
    let mut term = Terminal::new(80, 24);

    // XTVERSION: CSI > q
    term.process(b"\x1b[>q");
    let response = term.drain_responses();
    let response_str = std::str::from_utf8(&response).unwrap();
    assert!(
        response_str.starts_with("\x1bP>|par-term("),
        "XTVERSION should respond with par-term version"
    );
    assert!(
        response_str.ends_with(")\x1b\\"),
        "XTVERSION should end with ST"
    );
}

// ========== Scroll Region and Tab Tests ==========

#[test]
fn test_scroll_region() {
    let mut term = Terminal::new(80, 24);

    // Set scroll region rows 6-16 (1-indexed)
    term.process(b"\x1b[6;16r");
    assert_eq!(term.scroll_region_top, 5); // 0-indexed
    assert_eq!(term.scroll_region_bottom, 15);

    // Reset to full screen
    term.process(b"\x1b[r");
    assert_eq!(term.scroll_region_top, 0);
    assert_eq!(term.scroll_region_bottom, 23);
}

#[test]
fn test_tab_stops() {
    let mut term = Terminal::new(80, 24);

    // Set a tab stop
    term.process(b"\x1b[1;20H");
    term.process(b"\x1bH"); // HTS (ESC H)
    assert!(term.tab_stops[19]); // 0-indexed

    // Clear tab at current position
    term.process(b"\x1b[g"); // or \x1b[0g
    assert!(!term.tab_stops[19]);

    // Clear all tabs
    term.process(b"\x1b[3g");
    assert!(!term.tab_stops.iter().any(|&x| x));
}

// ========== Cursor Style Tests ==========

#[test]
fn test_cursor_style() {
    let mut term = Terminal::new(80, 24);

    // Blinking block
    term.process(b"\x1b[1 q");
    assert_eq!(term.cursor.style, CursorStyle::BlinkingBlock);

    // Steady block
    term.process(b"\x1b[2 q");
    assert_eq!(term.cursor.style, CursorStyle::SteadyBlock);

    // Blinking underline
    term.process(b"\x1b[3 q");
    assert_eq!(term.cursor.style, CursorStyle::BlinkingUnderline);

    // Steady underline
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor.style, CursorStyle::SteadyUnderline);

    // Blinking bar
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor.style, CursorStyle::BlinkingBar);

    // Steady bar
    term.process(b"\x1b[6 q");
    assert_eq!(term.cursor.style, CursorStyle::SteadyBar);
}

// ========== Save/Restore Cursor Tests ==========

#[test]
fn test_save_restore_cursor_ansi() {
    let mut term = Terminal::new(80, 24);

    term.process(b"\x1b[10;15H");
    term.process(b"\x1b[31m"); // Red fg

    // Save cursor (ANSI.SYS style)
    term.process(b"\x1b[s");

    // Move and change
    term.process(b"\x1b[20;5H");
    term.process(b"\x1b[32m");

    // Restore cursor
    term.process(b"\x1b[u");
    assert_eq!(term.cursor.col, 14); // 0-indexed
    assert_eq!(term.cursor.row, 9);
}

// ========== XTWINOPS Tests ==========

#[test]
fn test_xtwinops_report_size() {
    let mut term = Terminal::new(80, 24);

    // Report text area size (CSI 18 t)
    term.process(b"\x1b[18t");
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b[8;24;80t");
}

#[test]
fn test_xtwinops_title_stack() {
    let mut term = Terminal::new(80, 24);

    term.process(b"\x1b]0;Original\x1b\\");

    // Save title (CSI 22 t)
    term.process(b"\x1b[22t");

    term.process(b"\x1b]0;New\x1b\\");

    // Restore title (CSI 23 t)
    term.process(b"\x1b[23t");
    assert_eq!(term.title(), "Original");
}

// ========== Insert Mode Tests ==========

#[test]
fn test_insert_mode() {
    let mut term = Terminal::new(80, 24);

    // Enable insert mode (IRM)
    term.process(b"\x1b[4h");
    assert!(term.insert_mode);

    // Disable insert mode
    term.process(b"\x1b[4l");
    assert!(!term.insert_mode);
}

// ========== Mode Changed Event Tests ==========

/// Helper: drain events and find ModeChanged events matching a mode name
fn find_mode_events(term: &mut Terminal, mode: &str) -> Vec<(String, bool)> {
    term.poll_events()
        .into_iter()
        .filter_map(|e| match e {
            TerminalEvent::ModeChanged(m, enabled) if m == mode => Some((m, enabled)),
            _ => None,
        })
        .collect()
}

#[test]
fn test_mode_changed_event_mouse_normal() {
    let mut term = Terminal::new(80, 24);
    term.poll_events(); // Clear any initial events

    // Enable normal mouse tracking
    term.process(b"\x1b[?1000h");
    let events = find_mode_events(&mut term, "mouse_normal");
    assert_eq!(events.len(), 1);
    assert!(events[0].1); // enabled = true

    // Disable mouse tracking
    term.process(b"\x1b[?1000l");
    let events = find_mode_events(&mut term, "mouse_normal");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1); // enabled = false
}

#[test]
fn test_mode_changed_event_mouse_button_event() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?1002h");
    let events = find_mode_events(&mut term, "mouse_button_event");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?1002l");
    let events = find_mode_events(&mut term, "mouse_button_event");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_mouse_any_event() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?1003h");
    let events = find_mode_events(&mut term, "mouse_any_event");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?1003l");
    let events = find_mode_events(&mut term, "mouse_any_event");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_mouse_encoding() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    // Enable SGR encoding
    term.process(b"\x1b[?1006h");
    let events = find_mode_events(&mut term, "mouse_sgr");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    // Disable SGR encoding
    term.process(b"\x1b[?1006l");
    let events = find_mode_events(&mut term, "mouse_sgr");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_bracketed_paste() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?2004h");
    let events = find_mode_events(&mut term, "bracketed_paste");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?2004l");
    let events = find_mode_events(&mut term, "bracketed_paste");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_application_cursor() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?1h");
    let events = find_mode_events(&mut term, "application_cursor");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?1l");
    let events = find_mode_events(&mut term, "application_cursor");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_focus_tracking() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?1004h");
    let events = find_mode_events(&mut term, "focus_tracking");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?1004l");
    let events = find_mode_events(&mut term, "focus_tracking");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_cursor_visible() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    // Hide cursor
    term.process(b"\x1b[?25l");
    let events = find_mode_events(&mut term, "cursor_visible");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);

    // Show cursor
    term.process(b"\x1b[?25h");
    let events = find_mode_events(&mut term, "cursor_visible");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);
}

#[test]
fn test_mode_changed_event_alternate_screen() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    // Enter alt screen
    term.process(b"\x1b[?1049h");
    let events = find_mode_events(&mut term, "alternate_screen");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    // Leave alt screen
    term.process(b"\x1b[?1049l");
    let events = find_mode_events(&mut term, "alternate_screen");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_origin_mode() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[?6h");
    let events = find_mode_events(&mut term, "origin_mode");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[?6l");
    let events = find_mode_events(&mut term, "origin_mode");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_event_auto_wrap() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    // Disable auto-wrap (default is on)
    term.process(b"\x1b[?7l");
    let events = find_mode_events(&mut term, "auto_wrap");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);

    // Re-enable auto-wrap
    term.process(b"\x1b[?7h");
    let events = find_mode_events(&mut term, "auto_wrap");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);
}

#[test]
fn test_mode_changed_event_insert_mode() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    term.process(b"\x1b[4h");
    let events = find_mode_events(&mut term, "insert_mode");
    assert_eq!(events.len(), 1);
    assert!(events[0].1);

    term.process(b"\x1b[4l");
    let events = find_mode_events(&mut term, "insert_mode");
    assert_eq!(events.len(), 1);
    assert!(!events[0].1);
}

#[test]
fn test_mode_changed_no_event_when_mouse_already_off() {
    let mut term = Terminal::new(80, 24);
    term.poll_events();

    // Mouse is already off, resetting should not emit an event
    term.process(b"\x1b[?1000l");
    let all_events: Vec<_> = term
        .poll_events()
        .into_iter()
        .filter(|e| matches!(e, TerminalEvent::ModeChanged(..)))
        .collect();
    assert!(all_events.is_empty());
}
