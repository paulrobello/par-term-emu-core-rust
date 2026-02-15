// Keyboard protocol tests
use crate::terminal::*;

#[test]
fn test_kitty_keyboard_query() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[?u");
    assert_eq!(term.drain_responses(), b"\x1b[?0u");
}

#[test]
fn test_kitty_keyboard_set_mode() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[=1;1u");
    assert_eq!(term.keyboard_flags(), 1);
}

#[test]
fn test_modify_other_keys_mode_setting() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[>4;2m");
    assert_eq!(term.modify_other_keys_mode(), 2);
}
