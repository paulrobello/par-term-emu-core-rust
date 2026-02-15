// Basic terminal tests
use crate::color::Color;
use crate::terminal::*;

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
    assert_eq!(cell.fg, Color::Rgb(255, 128, 64));
}

#[test]
fn test_default_tab_stops() {
    let term = Terminal::new(80, 24);
    assert!(term.tab_stops[0]);
    assert!(term.tab_stops[8]);
    assert!(term.tab_stops[16]);
    assert!(!term.tab_stops[1]);
}

#[test]
fn test_set_tab_stop() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5G"); // Move to column 5
    term.process(b"\x1bH"); // Set tab stop (HTS)

    assert!(term.tab_stops[4]); // 0-indexed
}

#[test]
fn test_clear_tab_stop() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[8G"); // Move to column 8 (tab stop)
    term.process(b"\x1b[0g"); // Clear tab stop at current position

    assert!(!term.tab_stops[7]); // 0-indexed, should be cleared
}

#[test]
fn test_clear_all_tab_stops() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[3g"); // Clear all tab stops

    assert!(term.tab_stops.iter().all(|&x| !x));
}
