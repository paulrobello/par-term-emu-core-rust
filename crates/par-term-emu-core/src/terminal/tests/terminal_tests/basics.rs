//! Terminal construction, text, colors, screens, mouse and focus modes.

use super::*;

#[test]
fn test_terminal_creation() {
    let term = Terminal::new(80, 24);
    assert_eq!(term.size(), (80, 24));
}

#[test]
fn test_write_simple_text() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello");

    let content = term.content();
    assert!(content.starts_with("Hello"));
}

#[test]
fn test_newline() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Line1\nLine2");

    let content = term.content();
    let lines: Vec<&str> = content.lines().collect();
    assert!(lines[0].starts_with("Line1"));
    // LF alone doesn't reset column, so Line2 appears after Line1's cursor position
    assert!(lines[1].contains("Line2"));
}

#[test]
fn test_true_color() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[38;2;255;128;64mTrue Color\x1b[0m");

    let content = term.content();
    assert!(content.contains("True Color"));

    // Check the color was set correctly
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.fg(), Color::Rgb(255, 128, 64));
}

#[test]
fn test_alt_screen() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Primary");

    // Switch to alt screen
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());

    term.process(b"Alternate");
    let content = term.content();
    assert!(content.contains("Alternate"));
    assert!(!content.contains("Primary"));

    // Switch back
    term.process(b"\x1b[?1049l");
    assert!(!term.is_alt_screen_active());

    let content = term.content();
    assert!(content.contains("Primary"));
}

#[test]
fn test_mouse_modes() {
    let mut term = Terminal::new(80, 24);

    // Enable normal mouse tracking
    term.process(b"\x1b[?1000h");
    assert_eq!(term.mouse_mode(), MouseMode::Normal);

    // Enable SGR encoding
    term.process(b"\x1b[?1006h");
    assert_eq!(term.mouse_encoding(), MouseEncoding::Sgr);

    // Disable mouse
    term.process(b"\x1b[?1000l");
    assert_eq!(term.mouse_mode(), MouseMode::Off);
}

#[test]
fn test_bracketed_paste() {
    let mut term = Terminal::new(80, 24);

    assert!(!term.bracketed_paste());

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");
    assert!(term.bracketed_paste());

    // Disable
    term.process(b"\x1b[?2004l");
    assert!(!term.bracketed_paste());
}

#[test]
fn test_focus_tracking() {
    let mut term = Terminal::new(80, 24);

    assert!(!term.focus_tracking());

    // Enable focus tracking
    term.process(b"\x1b[?1004h");
    assert!(term.focus_tracking());

    // Test focus events
    let focus_in = term.report_focus_in();
    assert_eq!(focus_in, b"\x1b[I");

    let focus_out = term.report_focus_out();
    assert_eq!(focus_out, b"\x1b[O");
}

#[test]
fn test_shell_integration() {
    let mut term = Terminal::new(80, 24);
    // audit SEC-005: OSC 7 tracking is opt-in by default; tests exercise the opt-in path
    term.set_accept_osc7(true);

    // Prompt start
    term.process(b"\x1b]133;A\x07");
    assert!(term.shell_integration().in_prompt());

    // Command start
    term.process(b"\x1b]133;B\x07");
    assert!(term.shell_integration().in_command_input());

    // Command executed
    term.process(b"\x1b]133;C\x07");
    assert!(term.shell_integration().in_command_output());

    // Set CWD (OSC 7 with file:// URL format)
    term.process(b"\x1b]7;file://hostname/home/user\x07");
    assert_eq!(term.shell_integration().cwd(), Some("/home/user"));
}

#[test]
fn test_mouse_event_encoding() {
    let mut term = Terminal::new(80, 24);
    term.set_mouse_mode(MouseMode::Normal);
    term.set_mouse_encoding(MouseEncoding::Sgr);

    let event = MouseEvent::new(0, 10, 5, true, 0);
    let encoded = term.report_mouse(event);

    assert_eq!(encoded, b"\x1b[<0;11;6M");
}
