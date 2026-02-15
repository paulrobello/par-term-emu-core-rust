// Cursor movement tests
use crate::terminal::*;

#[test]
fn test_tab_forward() {
    let mut term = Terminal::new(80, 24);
    term.process(b"A\t"); // Write A then tab

    assert_eq!(term.cursor.col, 8); // Should tab to column 8
}

#[test]
fn test_cursor_movement_param_zero() {
    let mut term = Terminal::new(80, 24);
    term.cursor.goto(5, 5);

    term.process(b"\x1b[C");
    assert_eq!(term.cursor.col, 6);

    term.process(b"\x1b[D");
    assert_eq!(term.cursor.col, 5);

    term.process(b"\x1b[A");
    assert_eq!(term.cursor.row, 4);

    term.process(b"\x1b[B");
    assert_eq!(term.cursor.row, 5);

    term.cursor.goto(5, 5);
    term.process(b"\x1b[0C");
    assert_eq!(term.cursor.col, 6);

    term.process(b"\x1b[0D");
    assert_eq!(term.cursor.col, 5);

    term.process(b"\x1b[3C");
    assert_eq!(term.cursor.col, 8);
}

#[test]
fn test_cursor_forward_tabulation() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[2I");
    assert_eq!(term.cursor.col, 16);
}

#[test]
fn test_cursor_backward_tabulation() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[20G");
    term.process(b"\x1b[1Z");
    assert_eq!(term.cursor.col, 16);
}

#[test]
fn test_cursor_bounds_checking() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[999;999H");
    assert_eq!(term.cursor.col, 79);
    assert_eq!(term.cursor.row, 23);
}

#[test]
fn test_save_restore_cursor() {
    use crate::color::{Color, NamedColor};
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[10;20H");
    term.process(b"\x1b[31m");
    term.process(b"\x1b[s");

    term.process(b"\x1b[1;1H");
    term.process(b"\x1b[0m");

    term.process(b"\x1b[u");

    assert_eq!(term.cursor.col, 19);
    assert_eq!(term.cursor.row, 9);
    assert_eq!(term.fg, Color::Named(NamedColor::Red));
}

#[test]
fn test_cursor_next_previous_line() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5;10H");
    term.process(b"\x1b[2E");
    assert_eq!(term.cursor.row, 6);
    assert_eq!(term.cursor.col, 0);

    term.process(b"\x1b[1F");
    assert_eq!(term.cursor.row, 5);
    assert_eq!(term.cursor.col, 0);
}

#[test]
fn test_cursor_horizontal_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[42G");
    assert_eq!(term.cursor.col, 41);
}

#[test]
fn test_line_position_absolute() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[12d");
    assert_eq!(term.cursor.row, 11);
}

#[test]
fn test_reverse_index() {
    let mut term = Terminal::new(80, 10);
    term.process(b"Line 0\nLine 1\nLine 2\nLine 3");
    term.process(b"\x1b[1;1H");
    term.process(b"\x1bM");

    let line0 = term.grid().row(0).unwrap();
    let text: String = line0.iter().map(|c| c.c).collect();
    assert!(text.trim().is_empty());
}

#[test]
fn test_cursor_style_blinking_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
    term.process(b"\x1b[1 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
}

#[test]
fn test_cursor_style_steady_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[2 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBlock);
}

#[test]
fn test_cursor_style_blinking_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[3 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingUnderline);
}

#[test]
fn test_cursor_style_steady_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);
}

#[test]
fn test_cursor_style_blinking_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}

#[test]
fn test_cursor_style_steady_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[6 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBar);
}

#[test]
fn test_cursor_style_saved_restored() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
    term.process(b"\x1b7");
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);
    term.process(b"\x1b8");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}
