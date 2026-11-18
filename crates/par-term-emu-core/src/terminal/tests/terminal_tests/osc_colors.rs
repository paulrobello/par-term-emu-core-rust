//! OSC 10/11/12 default colors and their queries.

use super::*;

#[test]
fn test_osc_10_query_default_fg() {
    let mut term = Terminal::new(80, 24);

    // Query default foreground color: OSC 10 ; ? ST
    term.process(b"\x1b]10;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 10 query");

    let response = String::from_utf8_lossy(&responses);
    // Default fg is White (192, 192, 192) = 0xc0c0 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]10;"),
        "Response should start with OSC 10"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_11_query_default_bg() {
    let mut term = Terminal::new(80, 24);

    // Query default background color: OSC 11 ; ? ST
    term.process(b"\x1b]11;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 11 query");

    let response = String::from_utf8_lossy(&responses);
    // Default bg is Black (0, 0, 0) = 0x0000 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]11;"),
        "Response should start with OSC 11"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_12_query_cursor_color() {
    let mut term = Terminal::new(80, 24);

    // Query cursor color: OSC 12 ; ? ST
    term.process(b"\x1b]12;?\x1b\\");

    // Check response
    let responses = term.drain_responses();
    assert!(!responses.is_empty(), "Expected response from OSC 12 query");

    let response = String::from_utf8_lossy(&responses);
    // Default cursor color is White (192, 192, 192) = 0xc0c0 in 16-bit
    assert!(
        response.contains("rgb:"),
        "Response should contain rgb: format"
    );
    assert!(
        response.starts_with("\x1b]12;"),
        "Response should start with OSC 12"
    );
    assert!(response.ends_with("\x1b\\"), "Response should end with ST");
}

#[test]
fn test_osc_10_11_12_custom_colors() {
    let mut term = Terminal::new(80, 24);

    // Set custom colors
    term.set_default_fg(Color::Rgb(255, 128, 64));
    term.set_default_bg(Color::Rgb(32, 64, 128));
    term.set_cursor_color(Color::Rgb(0, 255, 0));

    // Query foreground
    term.process(b"\x1b]10;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 255 * 257 = 65535 = 0xffff, 128 * 257 = 32896 = 0x8080, 64 * 257 = 16448 = 0x4040
    assert!(response.contains("ffff"), "Should contain red=255");
    assert!(response.contains("8080"), "Should contain green=128");
    assert!(response.contains("4040"), "Should contain blue=64");

    // Query background
    term.process(b"\x1b]11;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 32 * 257 = 8224 = 0x2020, 64 * 257 = 16448 = 0x4040, 128 * 257 = 32896 = 0x8080
    assert!(response.contains("2020"), "Should contain red=32");
    assert!(response.contains("4040"), "Should contain green=64");
    assert!(response.contains("8080"), "Should contain blue=128");

    // Query cursor color
    term.process(b"\x1b]12;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);
    // 0 * 257 = 0 = 0x0000, 255 * 257 = 65535 = 0xffff
    assert!(response.contains("0000"), "Should contain red=0");
    assert!(response.contains("ffff"), "Should contain green=255");
}

#[test]
fn test_osc_color_query_response_format() {
    let mut term = Terminal::new(80, 24);

    // Set a known color (pure red)
    term.set_default_fg(Color::Rgb(255, 0, 0));

    // Query and check exact format
    term.process(b"\x1b]10;?\x1b\\");
    let responses = term.drain_responses();
    let response = String::from_utf8_lossy(&responses);

    // Expected format: ESC ] 10 ; rgb:ffff/0000/0000 ESC \
    // 255 * 257 = 65535 = 0xffff, 0 * 257 = 0 = 0x0000
    assert_eq!(response, "\x1b]10;rgb:ffff/0000/0000\x1b\\");
}

#[test]
fn test_osc_color_getters_setters() {
    let mut term = Terminal::new(80, 24);

    // Test default colors
    assert_eq!(term.default_fg(), Color::Named(NamedColor::White));
    assert_eq!(term.default_bg(), Color::Named(NamedColor::Black));
    assert_eq!(term.cursor_color(), Color::Named(NamedColor::White));

    // Test setters
    term.set_default_fg(Color::Rgb(100, 150, 200));
    term.set_default_bg(Color::Rgb(50, 75, 100));
    term.set_cursor_color(Color::Rgb(255, 255, 0));

    // Verify getters
    assert_eq!(term.default_fg(), Color::Rgb(100, 150, 200));
    assert_eq!(term.default_bg(), Color::Rgb(50, 75, 100));
    assert_eq!(term.cursor_color(), Color::Rgb(255, 255, 0));

    // Verify RGB conversion
    assert_eq!(term.default_fg().to_rgb(), (100, 150, 200));
    assert_eq!(term.default_bg().to_rgb(), (50, 75, 100));
    assert_eq!(term.cursor_color().to_rgb(), (255, 255, 0));
}

#[test]
fn test_osc_color_queries_multiple() {
    let mut term = Terminal::new(80, 24);

    // Query all three colors in sequence
    term.process(b"\x1b]10;?\x1b\\");
    term.process(b"\x1b]11;?\x1b\\");
    term.process(b"\x1b]12;?\x1b\\");

    // Drain responses - should have 3 responses
    let responses = term.drain_responses();
    let response_str = String::from_utf8_lossy(&responses);

    // Count OSC responses (should have 3)
    let osc_count = response_str.matches("\x1b]").count();
    assert_eq!(osc_count, 3, "Should have 3 OSC responses");

    // Verify each type is present
    assert!(response_str.contains("\x1b]10;"), "Missing OSC 10 response");
    assert!(response_str.contains("\x1b]11;"), "Missing OSC 11 response");
    assert!(response_str.contains("\x1b]12;"), "Missing OSC 12 response");
}
