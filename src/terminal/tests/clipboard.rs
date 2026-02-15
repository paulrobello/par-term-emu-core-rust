// Clipboard (OSC 52) tests
use crate::terminal::*;

#[test]
fn test_osc52_clipboard_write() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]52;c;SGVsbG8sIFdvcmxkIQ==\x1b\\");
    assert_eq!(term.clipboard(), Some("Hello, World!"));
}
