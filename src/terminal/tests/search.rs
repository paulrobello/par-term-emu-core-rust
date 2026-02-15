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
