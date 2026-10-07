use crate::ffi::SharedState;
use crate::terminal::Terminal;
use std::ffi::CStr;

#[test]
fn test_shared_state_dimensions() {
    let term = Terminal::new(80, 24);
    let state = SharedState::from_terminal(&term);
    assert_eq!(state.cols, 80);
    assert_eq!(state.rows, 24);
    assert_eq!(state.cell_count, 80 * 24);
}

#[test]
fn test_shared_state_cursor() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[6;11H"); // Move to row 6, col 11 (1-indexed) = (10, 5) 0-indexed
    let state = SharedState::from_terminal(&term);
    assert_eq!(state.cursor_col, 10);
    assert_eq!(state.cursor_row, 5);
    assert!(state.cursor_visible);
}

#[test]
fn test_shared_state_title() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b]0;Test Title\x07");
    let state = SharedState::from_terminal(&term);
    let title = unsafe { CStr::from_ptr(state.title) };
    assert_eq!(title.to_str().unwrap(), "Test Title");
    assert_eq!(state.title_len, 10);
}

#[test]
fn test_shared_state_cell_content() {
    let mut term = Terminal::new(80, 24);
    term.process(b"ABC");
    let state = SharedState::from_terminal(&term);
    assert!(!state.cells.is_null());
    unsafe {
        let cell0 = &*state.cells;
        let text = std::str::from_utf8(&cell0.text[..cell0.text_len as usize]).unwrap();
        assert_eq!(text, "A");
        assert_eq!(cell0.width, 1);

        let cell1 = &*state.cells.add(1);
        let text1 = std::str::from_utf8(&cell1.text[..cell1.text_len as usize]).unwrap();
        assert_eq!(text1, "B");
    }
}

#[test]
fn test_shared_state_alt_screen() {
    let mut term = Terminal::new(80, 24);
    assert!(!SharedState::from_terminal(&term).alt_screen_active);
    term.process(b"\x1b[?1049h"); // Enter alt screen
    assert!(SharedState::from_terminal(&term).alt_screen_active);
}

#[test]
fn test_shared_state_drop_safety() {
    let term = Terminal::new(80, 24);
    let state = SharedState::from_terminal(&term);
    drop(state); // Should not panic or leak
}
