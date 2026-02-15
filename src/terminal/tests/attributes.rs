// Character attributes and SGR tests
use crate::color::{Color, NamedColor};
use crate::terminal::*;

#[test]
fn test_256_color() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[38;5;196mRed");

    let cell = term.grid().get(0, 0).unwrap();
    assert_eq!(cell.fg, Color::from_ansi_code(196));
}

#[test]
fn test_sgr_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;31;42mTest");
    term.process(b"\x1b[0m");

    assert_eq!(term.fg, Color::Named(NamedColor::White));
    assert_eq!(term.bg, Color::Named(NamedColor::Black));
    assert!(!term.flags.bold());
}

#[test]
fn test_multiple_sgr_attributes() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;3;4;9mTest");

    assert!(term.flags.bold());
    assert!(term.flags.italic());
    assert!(term.flags.underline());
    assert!(term.flags.strikethrough());
}

#[test]
fn test_underline_style_straight() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4mTest");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Straight);
}

#[test]
fn test_underline_style_double() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4:2mDouble");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);
}

#[test]
fn test_underline_style_curly() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4:3mError");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Curly);
}

#[test]
fn test_underline_style_dotted() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4:4mDotted");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dotted);
}

#[test]
fn test_underline_style_dashed() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[4:5mDashed");
    let cell = term.active_grid().get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Dashed);
}
