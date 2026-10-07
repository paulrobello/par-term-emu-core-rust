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

#[test]
fn test_add_bookmark_cap_evicts_oldest() {
    let mut term = Terminal::with_scrollback(80, 24, 100);

    for _ in 0..1200 {
        term.add_bookmark(0, None);
    }

    let bookmarks = term.get_bookmarks();
    assert_eq!(
        bookmarks.len(),
        1000,
        "bookmarks must be capped at MAX_BOOKMARKS"
    );
    // Oldest evicted: ids 0..199 are gone and the newest 1000 survive.
    assert_eq!(bookmarks[0].id, 200);
    assert_eq!(bookmarks[999].id, 1199);
}
