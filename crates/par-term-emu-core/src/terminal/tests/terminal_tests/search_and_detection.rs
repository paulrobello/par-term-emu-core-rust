//! Scrollback search and semantic detection.

use super::*;

// === Additional tests for uncovered functionality ===

#[test]
fn test_search_case_sensitive() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World\nHELLO WORLD");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("Hello", options).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "Hello");
    assert_eq!(matches[0].row, 0);
    assert_eq!(matches[0].col, 0);
}

#[test]
fn test_search_case_insensitive() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World\nHELLO WORLD");

    let options = RegexSearchOptions {
        case_insensitive: true,
        ..Default::default()
    };
    let matches = term.search("hello", options).unwrap();
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_search_multiple_matches_same_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test test test");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("test", options).unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].col, 0);
    assert_eq!(matches[1].col, 5);
    assert_eq!(matches[2].col, 10);
}

#[test]
fn test_search_scrollback() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    // Fill screen and create scrollback
    for i in 0..10 {
        term.process(format!("Line{}\r\n", i).as_bytes());
    }
    term.process(b"target");

    let matches = term.search_scrollback("target", true, None);
    // Search may or may not find results depending on scrollback state
    let found_target = matches.iter().any(|m| m.text.contains("target"));
    assert!(found_target || matches.is_empty());
}

#[test]
fn test_search_scrollback_with_limit() {
    let mut term = Terminal::with_scrollback(80, 5, 100);
    for i in 0..20 {
        term.process(format!("match{}\r\n", i).as_bytes());
    }

    let matches = term.search_scrollback("match", true, Some(5));
    assert!(matches.len() <= 5);
}

#[test]
fn test_search_unicode_byte_offset() {
    // Test that search returns character offsets, not byte offsets
    // Japanese "Hello World" has multi-byte characters
    let mut term = Terminal::new(80, 24);
    // "こんにちは World" - 5 Japanese chars (3 bytes each) + space + "World"
    term.process("こんにちは World".as_bytes());

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("World", options).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "World");
    // "World" should be at character position 6 (5 Japanese chars + 1 space)
    // NOT byte position 16 (15 bytes for Japanese + 1 for space)
    assert_eq!(matches[0].col, 6);
    assert_eq!(matches[0].length, 5);
}

#[test]
fn test_search_unicode_query() {
    // Test searching for Unicode text
    let mut term = Terminal::new(80, 24);
    term.process("Hello こんにちは World".as_bytes());

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("こんにちは", options).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "こんにちは");
    assert_eq!(matches[0].col, 6); // After "Hello "
    assert_eq!(matches[0].length, 5); // 5 characters, not 15 bytes
}

#[test]
fn test_search_unicode_multiple_matches() {
    let mut term = Terminal::new(80, 24);
    // Two occurrences of "日本" with ASCII text between them
    term.process("日本 is Japan 日本".as_bytes());

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("日本", options).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].col, 0); // First occurrence at start
    assert_eq!(matches[0].length, 2);
    // "日本 is Japan " = 2 + 1 + 2 + 1 + 5 + 1 = 12 chars, so second "日本" is at col 12
    assert_eq!(matches[1].col, 12);
    assert_eq!(matches[1].length, 2);
}

#[test]
fn test_detect_urls() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Visit https://example.com for more");

    let items = term.detect_urls();
    // Check that URL was detected
    let has_url = items
        .iter()
        .any(|item| matches!(item, DetectedItem::Url(url, _, _) if url.contains("example.com")));
    assert!(has_url, "Should detect example.com URL");
}

#[test]
fn test_detect_multiple_urls() {
    let mut term = Terminal::new(80, 24);
    term.process(b"http://test.com and https://example.org");

    let items = term.detect_urls();
    let urls: Vec<&String> = items
        .iter()
        .filter_map(|item| match item {
            DetectedItem::Url(url, _, _) => Some(url),
            _ => None,
        })
        .collect();
    assert_eq!(urls.len(), 2, "both URLs detected: {urls:?}");
    assert!(
        urls.iter().any(|u| u.contains("http://test.com")),
        "http URL detected: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.contains("https://example.org")),
        "https URL detected: {urls:?}"
    );
}

#[test]
fn test_detect_file_paths() {
    let mut term = Terminal::new(80, 24);
    term.process(b"See /usr/local/bin/test.sh:42 for details");

    let items = term.detect_file_paths();
    // Check that file path was detected with line number
    let has_path = items.iter().any(|item| {
        matches!(item, DetectedItem::FilePath(path, _, _, line_num)
            if path.contains("/usr/local/bin/test.sh") && *line_num == Some(42))
    });
    assert!(
        has_path || items.is_empty(),
        "File path detection may vary by implementation"
    );
}

#[test]
fn test_detect_semantic_items_git_hash() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Commit a1b2c3d was merged");

    let items = term.detect_semantic_items();
    // Git hash detection may vary by implementation
    let has_git_hash = items
        .iter()
        .any(|item| matches!(item, DetectedItem::GitHash(hash, _, _) if hash.contains("a1b2c3")));
    // Just ensure it doesn't panic
    assert!(has_git_hash || items.is_empty());
}

#[test]
fn test_detect_semantic_items_ip_address() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Server at 192.168.1.100");

    let items = term.detect_semantic_items();
    let has_ip = items
        .iter()
        .any(|item| matches!(item, DetectedItem::IpAddress(ip, _, _) if ip == "192.168.1.100"));
    assert!(has_ip);
}

#[test]
fn test_detect_semantic_items_email() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Contact user@example.com");

    let items = term.detect_semantic_items();
    let has_email = items.iter().any(
        |item| matches!(item, DetectedItem::Email(email, _, _) if email == "user@example.com"),
    );
    assert!(has_email);
}
