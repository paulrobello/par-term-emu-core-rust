// Bookmark functionality tests
use crate::terminal::*;

#[test]
fn test_add_bookmark() {
    let mut term = Terminal::with_scrollback(80, 24, 100);
    term.process(b"Some content\r\n");

    let _id = term.add_bookmark(0, Some("Test Bookmark".to_string()));

    let bookmarks = term.get_bookmarks();
    assert!(!bookmarks.is_empty());
    assert_eq!(bookmarks[0].label, "Test Bookmark");
}
