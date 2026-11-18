//! OSC 52 clipboard read/write behavior.

use super::*;

#[test]
fn test_osc52_clipboard_write() {
    let mut term = Terminal::new(80, 24);

    // Initially clipboard should be empty
    assert_eq!(term.clipboard(), None);

    // Write to clipboard using OSC 52 with base64 encoded "Hello, World!"
    // Base64("Hello, World!") = "SGVsbG8sIFdvcmxkIQ=="
    term.process(b"\x1b]52;c;SGVsbG8sIFdvcmxkIQ==\x1b\\");

    // Clipboard should now contain the decoded text
    assert_eq!(term.clipboard(), Some("Hello, World!"));
}

#[test]
fn test_osc52_clipboard_query_allowed() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Test content".to_string()));

    // Enable clipboard read
    term.set_allow_clipboard_read(true);
    assert!(term.allow_clipboard_read());

    // Query clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should respond with base64 encoded content
    let response = term.drain_responses();
    // Base64("Test content") = "VGVzdCBjb250ZW50"
    assert_eq!(response, b"\x1b]52;c;VGVzdCBjb250ZW50\x1b\\");
}

#[test]
fn test_osc52_clipboard_query_denied() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Secret content".to_string()));

    // Clipboard read is disabled by default
    assert!(!term.allow_clipboard_read());

    // Query clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should NOT respond (security)
    let response = term.drain_responses();
    assert_eq!(response, b"");
}

#[test]
fn test_osc52_clipboard_write_allowed_by_default() {
    let mut term = Terminal::new(80, 24);

    assert!(term.allow_clipboard_write());

    // Base64("Hello") = "SGVsbG8="
    term.process(b"\x1b]52;c;SGVsbG8=\x1b\\");
    assert_eq!(term.clipboard(), Some("Hello"));
}

#[test]
fn test_osc52_clipboard_write_denied() {
    let mut term = Terminal::new(80, 24);
    term.set_clipboard(Some("Existing".to_string()));

    term.set_allow_clipboard_write(false);
    assert!(!term.allow_clipboard_write());

    // Write is ignored: existing content stays and nothing is replied.
    term.process(b"\x1b]52;c;SGVsbG8=\x1b\\");
    assert_eq!(term.clipboard(), Some("Existing"));
    assert_eq!(term.drain_responses(), b"");

    // The query path is independent of the write flag.
    term.set_allow_clipboard_read(true);
    term.process(b"\x1b]52;c;?\x1b\\");
    // Base64("Existing") = "RXhpc3Rpbmc="
    assert_eq!(term.drain_responses(), b"\x1b]52;c;RXhpc3Rpbmc=\x1b\\");
}

#[test]
fn test_osc52_clipboard_write_flag_survives_ris() {
    let mut term = Terminal::new(80, 24);
    term.set_allow_clipboard_write(false);

    term.process(b"\x1bc"); // RIS
    assert!(!term.allow_clipboard_write());

    term.process(b"\x1b]52;c;SGVsbG8=\x1b\\");
    assert_eq!(term.clipboard(), None);
}

#[test]
fn test_osc52_clipboard_empty_write_is_noop() {
    let mut term = Terminal::new(80, 24);

    // Set clipboard content
    term.set_clipboard(Some("Some text".to_string()));
    assert_eq!(term.clipboard(), Some("Some text"));

    // Empty clipboard payloads are treated as a no-op to avoid destructive
    // clears from zero-length OSC 52 writes (for example some mouse-click paths).
    term.process(b"\x1b]52;c;\x1b\\");

    // Clipboard should be preserved
    assert_eq!(term.clipboard(), Some("Some text"));
}

#[test]
fn test_osc52_clipboard_empty_query() {
    let mut term = Terminal::new(80, 24);

    // Enable clipboard read
    term.set_allow_clipboard_read(true);

    // Query empty clipboard
    term.process(b"\x1b]52;c;?\x1b\\");

    // Should respond with empty data
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b]52;c;\x1b\\");
}

#[test]
fn test_osc52_clipboard_multiple_operations() {
    let mut term = Terminal::new(80, 24);

    // Write first content
    term.process(b"\x1b]52;c;Zmlyc3Q=\x1b\\"); // "first"
    assert_eq!(term.clipboard(), Some("first"));

    // Overwrite with second content
    term.process(b"\x1b]52;c;c2Vjb25k\x1b\\"); // "second"
    assert_eq!(term.clipboard(), Some("second"));

    // Empty payload is a no-op (preserve existing content)
    term.process(b"\x1b]52;c;\x1b\\");
    assert_eq!(term.clipboard(), Some("second"));
}

#[test]
fn test_osc52_clipboard_programmatic_access() {
    let mut term = Terminal::new(80, 24);

    // Set via programmatic API
    term.set_clipboard(Some("API content".to_string()));
    assert_eq!(term.clipboard(), Some("API content"));

    // Clear via programmatic API
    term.set_clipboard(None);
    assert_eq!(term.clipboard(), None);
}

#[test]
fn test_osc52_clipboard_selection_parameter() {
    let mut term = Terminal::new(80, 24);

    // Test with explicit 'c' parameter (clipboard)
    term.process(b"\x1b]52;c;Y2xpcGJvYXJk\x1b\\"); // "clipboard"
    assert_eq!(term.clipboard(), Some("clipboard"));

    // Test with empty selection (should still work)
    term.process(b"\x1b]52;;cHJpbWFyeQ==\x1b\\"); // "primary"
    assert_eq!(term.clipboard(), Some("primary"));
}
