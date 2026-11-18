//! Underline styles and cursor shape styles.

use super::*;

#[test]
fn test_underline_style_straight() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4 - basic underline (should default to straight)
    term.process(b"\x1b[4mTest");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Straight);

    // SGR 4:1 - explicit straight underline
    term.process(b"\x1b[0m\x1b[4:1mTest");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Straight);
}

#[test]
fn test_underline_style_double() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:2 - double underline
    term.process(b"\x1b[4:2mDouble");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);
}

#[test]
fn test_underline_style_curly() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:3 - curly underline (for errors, spell check)
    term.process(b"\x1b[4:3mError");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Curly);
}

#[test]
fn test_underline_style_dotted() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:4 - dotted underline
    term.process(b"\x1b[4:4mDotted");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dotted);
}

#[test]
fn test_underline_style_dashed() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:5 - dashed underline
    term.process(b"\x1b[4:5mDashed");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dashed);
}

#[test]
fn test_underline_style_disable() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Set underline with style
    term.process(b"\x1b[4:3mText");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Curly);

    // SGR 24 - disable underline
    term.process(b"\x1b[24mNo");
    // Check cells written after disable (at position 4, 5)
    let cell = term.active_grid().get(4, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_reset() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Set underline with style
    term.process(b"\x1b[4:2mDouble");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);

    // SGR 0 - reset all
    term.process(b"\x1b[0mReset");
    // Check cells written after reset (at position 6, 7, etc)
    let cell = term.active_grid().get(6, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_none() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // SGR 4:0 - no underline (explicit)
    term.process(b"\x1b[4:0mNone");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_style_multiple_switches() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);

    // Start with straight
    term.process(b"\x1b[4mA");
    assert_eq!(
        term.active_grid().get(0, 0).unwrap().flags.underline_style,
        UnderlineStyle::Straight
    );

    // Switch to curly
    term.process(b"\x1b[4:3mB");
    assert_eq!(
        term.active_grid().get(1, 0).unwrap().flags.underline_style,
        UnderlineStyle::Curly
    );

    // Switch to double
    term.process(b"\x1b[4:2mC");
    assert_eq!(
        term.active_grid().get(2, 0).unwrap().flags.underline_style,
        UnderlineStyle::Double
    );

    // Disable
    term.process(b"\x1b[24mD");
    assert_eq!(
        term.active_grid().get(3, 0).unwrap().flags.underline_style,
        UnderlineStyle::None
    );
}

#[test]
fn test_cursor_style_blinking_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // Default should be blinking block
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);

    // CSI 1 SP q - blinking block
    term.process(b"\x1b[1 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
}

#[test]
fn test_cursor_style_steady_block() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 2 SP q - steady block
    term.process(b"\x1b[2 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBlock);
}

#[test]
fn test_cursor_style_blinking_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 3 SP q - blinking underline
    term.process(b"\x1b[3 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingUnderline);
}

#[test]
fn test_cursor_style_steady_underline() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 4 SP q - steady underline
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);
}

#[test]
fn test_cursor_style_blinking_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 5 SP q - blinking bar
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}

#[test]
fn test_cursor_style_steady_bar() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI 6 SP q - steady bar
    term.process(b"\x1b[6 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyBar);
}

#[test]
fn test_cursor_style_default() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // CSI SP q or CSI 0 SP q - default (blinking block)
    term.process(b"\x1b[ q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);

    term.process(b"\x1b[0 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBlock);
}

#[test]
fn test_cursor_style_saved_restored() {
    use crate::cursor::CursorStyle;
    let mut term = Terminal::new(80, 24);

    // Set to blinking bar
    term.process(b"\x1b[5 q");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);

    // Save cursor (DECSC - ESC 7)
    term.process(b"\x1b7");

    // Change to steady underline
    term.process(b"\x1b[4 q");
    assert_eq!(term.cursor().style(), CursorStyle::SteadyUnderline);

    // Restore cursor (DECRC - ESC 8)
    term.process(b"\x1b8");
    assert_eq!(term.cursor().style(), CursorStyle::BlinkingBar);
}
