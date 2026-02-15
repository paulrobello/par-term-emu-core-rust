// Search functionality tests
use crate::terminal::*;

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
fn test_search_regex_digits() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Error 404\nPort 8080\nVersion 3.14");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search(r"\d+", options).unwrap();
    // "3.14" matches as "3" and "14" separately (4 total matches)
    assert_eq!(matches.len(), 4);
    assert_eq!(matches[0].text, "404");
    assert_eq!(matches[1].text, "8080");
    assert_eq!(matches[2].text, "3");
    assert_eq!(matches[3].text, "14");
}

#[test]
fn test_search_regex_word_characters() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo_bar baz-qux");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search(r"\w+", options).unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].text, "foo_bar");
    assert_eq!(matches[1].text, "baz");
    assert_eq!(matches[2].text, "qux");
}

#[test]
fn test_search_no_match_returns_empty() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("NotFound", options).unwrap();
    assert_eq!(matches.len(), 0);
}

#[test]
fn test_search_multiple_matches_one_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo bar foo baz foo");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("foo", options).unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0].col, 0);
    assert_eq!(matches[1].col, 8);
    assert_eq!(matches[2].col, 16);
}

#[test]
fn test_search_match_in_scrollback() {
    let mut term = Terminal::new(80, 5);

    // Fill screen with content
    for i in 0..10 {
        term.process(format!("Line {}\n", i).as_bytes());
    }

    let options = RegexSearchOptions {
        case_insensitive: false,
        include_scrollback: true,
        ..Default::default()
    };
    let matches = term.search("Line", options).unwrap();
    // Should find "Line" in both scrollback and visible screen
    assert!(!matches.is_empty());
    // Verify we found Line in the output
    assert!(matches.iter().any(|m| m.text == "Line"));
}

#[test]
fn test_search_exclude_scrollback() {
    let mut term = Terminal::new(80, 5);

    // Fill screen with content that will scroll
    for i in 0..10 {
        term.process(format!("Line {}\n", i).as_bytes());
    }

    let options = RegexSearchOptions {
        case_insensitive: false,
        include_scrollback: false,
        ..Default::default()
    };
    let matches = term.search("Line 0", options).unwrap();
    // Line 0 is in scrollback, so should not be found
    assert_eq!(matches.len(), 0);
}

#[test]
fn test_search_unicode_characters() {
    let mut term = Terminal::new(80, 24);
    term.process("Hello 世界\nBonjour 世界".as_bytes());

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search("世界", options).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].text, "世界");
    assert_eq!(matches[1].text, "世界");
}

#[test]
fn test_search_unicode_case_insensitive() {
    let mut term = Terminal::new(80, 24);
    term.process("Café\ncafé\nCAFÉ".as_bytes());

    let options = RegexSearchOptions {
        case_insensitive: true,
        ..Default::default()
    };
    let matches = term.search("café", options).unwrap();
    // Should match all three variations
    assert_eq!(matches.len(), 3);
}

#[test]
fn test_search_empty_pattern_edge_case() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    // Empty pattern should match every position
    let matches = term.search("", options).unwrap();
    // Empty regex matches at every position between characters
    assert!(!matches.is_empty());
}

#[test]
fn test_search_max_matches() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo foo foo foo foo");

    let options = RegexSearchOptions {
        case_insensitive: false,
        max_matches: 2,
        ..Default::default()
    };
    let matches = term.search("foo", options).unwrap();
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_search_reverse() {
    let mut term = Terminal::new(80, 24);
    term.process(b"first\nsecond\nthird");

    let options = RegexSearchOptions {
        case_insensitive: false,
        reverse: true,
        ..Default::default()
    };
    let matches = term.search(r"\w+", options).unwrap();
    // Results should be in reverse order
    assert_eq!(matches[0].text, "third");
    assert_eq!(matches[1].text, "second");
    assert_eq!(matches[2].text, "first");
}

#[test]
fn test_search_captures_groups() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Error: 404 Not Found");

    let options = RegexSearchOptions {
        case_insensitive: false,
        ..Default::default()
    };
    let matches = term.search(r"Error:\s+(\d+)\s+(\w+)", options).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].captures.len(), 3); // Full match + 2 groups
    assert_eq!(matches[0].captures[0], "Error: 404 Not");
    assert_eq!(matches[0].captures[1], "404");
    assert_eq!(matches[0].captures[2], "Not");
}

#[test]
fn test_search_multiline_mode() {
    let mut term = Terminal::new(80, 24);
    term.process(b"start\nmiddle\nend");

    let options = RegexSearchOptions {
        case_insensitive: false,
        multiline: true,
        ..Default::default()
    };
    // Search for "middle" - multiline mode allows ^ and $ to match line boundaries
    let matches = term.search("middle", options).unwrap();
    assert!(!matches.is_empty());
    assert_eq!(matches[0].text, "middle");
}

#[test]
fn test_search_clear_matches() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let options = RegexSearchOptions::default();
    let matches = term.search("Hello", options).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(term.get_search_matches().len(), 1);

    term.clear_search_matches();
    assert_eq!(term.get_search_matches().len(), 0);
    assert!(term.get_current_regex_pattern().is_none());
}

#[test]
fn test_search_next_match() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo\nbar\nfoo\nbaz\nfoo");

    let options = RegexSearchOptions::default();
    let matches = term.search("foo", options).unwrap();

    // Should find multiple "foo" occurrences
    assert!(matches.len() >= 3);

    // All matches should have "foo" as text
    for m in matches.iter() {
        assert_eq!(m.text, "foo");
    }

    // Find next match from position (0, 0)
    let next = term.next_regex_match(0, 0);
    assert!(next.is_some());
    let next = next.unwrap();
    assert_eq!(next.text, "foo");

    // Find next match after first occurrence - should get a later row
    let next = term.next_regex_match(0, 2);
    assert!(next.is_some());
    assert_eq!(next.unwrap().text, "foo");
}

#[test]
fn test_search_prev_match() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo\nbar\nfoo\nbaz\nfoo");

    let options = RegexSearchOptions::default();
    term.search("foo", options).unwrap();

    // Find previous match from position (4, 0)
    let prev = term.prev_regex_match(4, 0);
    assert!(prev.is_some());
    let prev = prev.unwrap();
    assert_eq!(prev.text, "foo");
    assert_eq!(prev.row, 2);

    // Find previous match from row 2
    let prev = term.prev_regex_match(2, 0);
    assert!(prev.is_some());
    let prev = prev.unwrap();
    assert_eq!(prev.row, 0);
}

#[test]
fn test_search_text_basic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let matches = term.search_text("World", true);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "World");
    assert_eq!(matches[0].col, 6);
    assert_eq!(matches[0].row, 0);
}

#[test]
fn test_search_text_case_insensitive() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello World");

    let matches = term.search_text("world", false);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "World");
}

#[test]
fn test_search_scrollback_basic() {
    let mut term = Terminal::new(80, 3);

    // Fill screen and create scrollback
    for i in 0..5 {
        term.process(format!("Line {}\n", i).as_bytes());
    }

    let matches = term.search_scrollback("Line 0", true, None);
    assert_eq!(matches.len(), 1);
    // Scrollback rows have negative indices
    assert!(matches[0].row < 0);
    assert_eq!(matches[0].text, "Line 0");
}

#[test]
fn test_search_scrollback_max_lines() {
    let mut term = Terminal::new(80, 3);

    // Fill screen and create scrollback
    for i in 0..10 {
        term.process(format!("Line {}\n", i).as_bytes());
    }

    // Search only the most recent 2 scrollback lines
    let matches = term.search_scrollback("Line", true, Some(2));
    // Should find fewer matches than searching all scrollback
    assert!(!matches.is_empty());
    assert!(matches.len() <= 2);
}

#[test]
fn test_find_next_basic() {
    let mut term = Terminal::new(80, 24);
    term.process(b"foo bar foo baz");

    // find_text returns all matches in visible screen
    let all_matches = term.find_text("foo", true);
    assert!(all_matches.len() >= 2);
    assert_eq!(all_matches[0].text, "foo");
    assert_eq!(all_matches[1].text, "foo");

    // Test find_next - should find match after given position
    let match1 = term.find_next("foo", 0, 0, true);
    assert!(match1.is_some());
    assert_eq!(match1.unwrap().text, "foo");
}

#[test]
fn test_regex_search_alias() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test123");

    let options = RegexSearchOptions::default();
    let matches1 = term.search(r"\d+", options.clone()).unwrap();
    let matches2 = term.regex_search(r"\d+", options).unwrap();

    assert_eq!(matches1.len(), matches2.len());
    assert_eq!(matches1[0].text, matches2[0].text);
}

#[test]
fn test_get_regex_matches_alias() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test");

    let options = RegexSearchOptions::default();
    term.search("test", options).unwrap();

    let matches1 = term.get_search_matches();
    let matches2 = term.get_regex_matches();

    assert_eq!(matches1.len(), matches2.len());
}

#[test]
fn test_clear_regex_matches_alias() {
    let mut term = Terminal::new(80, 24);
    term.process(b"test");

    let options = RegexSearchOptions::default();
    term.search("test", options).unwrap();
    assert!(!term.get_search_matches().is_empty());

    term.clear_regex_matches();
    assert_eq!(term.get_search_matches().len(), 0);
}
