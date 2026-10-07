// URL and semantic item detection tests
use crate::terminal::*;

#[test]
fn test_detect_urls() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Visit https://example.com for more");

    let items = term.detect_urls();
    let has_url = items
        .iter()
        .any(|item| matches!(item, DetectedItem::Url(url, _, _) if url.contains("example.com")));
    assert!(has_url);
}
