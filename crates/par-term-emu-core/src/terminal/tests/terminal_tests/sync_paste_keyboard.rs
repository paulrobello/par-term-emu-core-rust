//! Synchronized updates, bracketed paste, and the kitty keyboard protocol.

use super::*;

#[test]
fn test_synchronized_updates() {
    let mut term = Terminal::new(80, 24);

    // Initially disabled
    assert!(!term.synchronized_updates());

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");
    assert!(term.synchronized_updates());

    // Process some content - it should be buffered
    term.process(b"Buffered");
    let content = term.content();
    // Content should be empty because it's buffered
    assert!(!content.contains("Buffered"));

    // Disable synchronized updates - this should flush the buffer
    term.process(b"\x1b[?2026l");
    assert!(!term.synchronized_updates());

    // Now content should appear
    let content = term.content();
    assert!(content.contains("Buffered"));
}

#[test]
fn test_synchronized_updates_multiple_updates() {
    let mut term = Terminal::new(80, 24);

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");

    // Send multiple updates
    term.process(b"Line1\r\n");
    term.process(b"Line2\r\n");
    term.process(b"Line3");

    // All should be buffered
    let content = term.content();
    assert!(!content.contains("Line1"));
    assert!(!content.contains("Line2"));
    assert!(!content.contains("Line3"));

    // Disable and flush
    term.process(b"\x1b[?2026l");

    // All lines should appear
    let content = term.content();
    assert!(content.contains("Line1"));
    assert!(content.contains("Line2"));
    assert!(content.contains("Line3"));
}

#[test]
fn test_synchronized_updates_manual_flush() {
    let mut term = Terminal::new(80, 24);

    // Enable synchronized updates
    term.process(b"\x1b[?2026h");
    term.process(b"Test");

    // Content buffered
    assert!(!term.content().contains("Test"));

    // Manual flush
    term.flush_synchronized_updates();

    // Content should appear, mode still enabled
    assert!(term.content().contains("Test"));
    assert!(term.synchronized_updates());
}

#[test]
fn test_paste_with_bracketed_mode() {
    let mut term = Terminal::new(80, 24);

    // Enable bracketed paste mode
    term.process(b"\x1b[?2004h");
    assert!(term.bracketed_paste());

    // Paste content
    term.paste("Hello World");

    // Content should appear with bracketed paste markers processed
    let content = term.content();
    assert!(content.contains("Hello World"));
}

#[test]
fn test_paste_without_bracketed_mode() {
    let mut term = Terminal::new(80, 24);

    // Bracketed paste mode disabled by default
    assert!(!term.bracketed_paste());

    // Paste content
    term.paste("Direct Paste");

    // Content should appear directly
    let content = term.content();
    assert!(content.contains("Direct Paste"));
}

#[test]
fn test_paste_multiline() {
    let mut term = Terminal::new(80, 24);

    // Enable bracketed paste
    term.process(b"\x1b[?2004h");

    // Paste multiline content
    term.paste("Line 1\nLine 2\nLine 3");

    // All lines should appear
    let content = term.content();
    assert!(content.contains("Line 1"));
    assert!(content.contains("Line 2"));
    assert!(content.contains("Line 3"));
}

#[test]
fn test_paste_special_characters() {
    let mut term = Terminal::new(80, 24);

    // Paste content with special characters
    term.paste("Tab:\tNewline:\nCarriage:\r");

    // Content should be processed correctly
    let content = term.content();
    assert!(content.contains("Tab:"));
    assert!(content.contains("Newline:"));
    assert!(content.contains("Carriage:"));
}

// Kitty Keyboard Protocol tests
#[test]
fn test_kitty_keyboard_query() {
    let mut term = Terminal::new(80, 24);

    // Query keyboard capabilities (should return 0 initially)
    term.process(b"\x1b[?u");
    assert_eq!(term.drain_responses(), b"\x1b[?0u");
}

#[test]
fn test_kitty_keyboard_set_mode() {
    let mut term = Terminal::new(80, 24);

    // Set keyboard mode with flags 1 (disambiguate)
    term.process(b"\x1b[=1;1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Query should return the set flags
    term.process(b"\x1b[?u");
    assert_eq!(term.drain_responses(), b"\x1b[?1u");

    // Set different flags (3 = disambiguate + report events)
    term.process(b"\x1b[=3;1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Unset (mode 0)
    term.process(b"\x1b[=0;0u");
    assert_eq!(term.keyboard_flags(), 0);
}

#[test]
fn test_kitty_keyboard_push_pop() {
    let mut term = Terminal::new(80, 24);

    // Set initial flags
    term.process(b"\x1b[=1;1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Push new flags to stack
    term.process(b"\x1b[>3u");
    assert_eq!(term.keyboard_flags(), 3);

    // Push another set
    term.process(b"\x1b[>7u");
    assert_eq!(term.keyboard_flags(), 7);

    // Pop once
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Pop again
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);

    // Popping empty stack should not change flags
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);
}

/// `CSI < u` without a count pops one entry: a program that pushes `CSI > 11 u`
/// and pops with a bare `CSI < u` on exit must not leave its flags on.
#[test]
fn test_kitty_keyboard_pop_without_a_count_pops_one() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>11u");
    assert_eq!(term.keyboard_flags(), 11);
    term.process(b"\x1b[<u");
    assert_eq!(term.keyboard_flags(), 0);

    term.process(b"\x1b[>3u\x1b[>7u");
    term.process(b"\x1b[<0u");
    assert_eq!(term.keyboard_flags(), 3, "a count of 0 pops one as well");
}

#[test]
fn test_kitty_keyboard_pop_multiple() {
    let mut term = Terminal::new(80, 24);

    // Set initial flags
    term.process(b"\x1b[=1;1u");

    // Push multiple values
    term.process(b"\x1b[>3u");
    term.process(b"\x1b[>7u");
    term.process(b"\x1b[>15u");
    assert_eq!(term.keyboard_flags(), 15);

    // Pop 2 at once
    term.process(b"\x1b[<2u");
    assert_eq!(term.keyboard_flags(), 3);
}

#[test]
fn test_kitty_keyboard_report_mode() {
    let mut term = Terminal::new(80, 24);

    // Set flags
    term.process(b"\x1b[=5;1u");
    assert_eq!(term.keyboard_flags(), 5);

    // Use mode 3 to report current flags
    term.process(b"\x1b[=0;3u");
    assert_eq!(term.drain_responses(), b"\x1b[?5u");
}

#[test]
fn test_kitty_keyboard_alternate_screen_stacks() {
    let mut term = Terminal::new(80, 24);

    // Set flags on main screen
    term.process(b"\x1b[=1;1u");
    term.process(b"\x1b[>3u");
    assert_eq!(term.keyboard_flags(), 3);

    // Switch to alternate screen
    term.process(b"\x1b[?1049h");
    assert!(term.is_alt_screen_active());

    // Push should use alternate stack
    term.process(b"\x1b[>7u");
    assert_eq!(term.keyboard_flags(), 7);

    // Pop from alternate stack
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 3);

    // Switch back to main screen
    term.process(b"\x1b[?1049l");
    assert!(!term.is_alt_screen_active());

    // Pop from main stack
    term.process(b"\x1b[<1u");
    assert_eq!(term.keyboard_flags(), 1);
}

#[test]
fn test_kitty_keyboard_reset() {
    let mut term = Terminal::new(80, 24);

    // Set flags
    term.process(b"\x1b[=15;1u");
    assert_eq!(term.keyboard_flags(), 15);

    // Reset terminal
    term.reset();

    // Flags should be reset to 0
    assert_eq!(term.keyboard_flags(), 0);
}
