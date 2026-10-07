// TUI-related features tests
use crate::terminal::*;

#[test]
fn test_dirty_region_tracking() {
    let mut term = Terminal::new(80, 24);
    term.process(b"Hello");
    let dirty = term.get_dirty_rows();
    assert!(dirty.contains(&0));
}

#[test]
fn test_export_text_basic() {
    let mut term = Terminal::new(20, 3);
    term.process(b"Hello\r\nWorld\r\nTest");
    let text = term.export_text();
    assert!(text.contains("Hello"));
    assert!(text.contains("World"));
    assert!(text.contains("Test"));
}
