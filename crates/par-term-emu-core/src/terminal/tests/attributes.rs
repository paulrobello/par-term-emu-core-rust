// Character attributes and SGR tests
use crate::color::{Color, NamedColor};
use crate::terminal::*;

#[test]
fn test_256_color() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[38;5;196mRed");

    let cell = term.grid().get(0, 0).unwrap();
    assert_eq!(cell.fg(), Color::from_ansi_code(196));
}

#[test]
fn test_sgr_reset() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;31;42mTest");
    term.process(b"\x1b[0m");

    assert_eq!(term.attrs.fg, Color::Named(NamedColor::White));
    assert_eq!(term.attrs.bg, Color::Named(NamedColor::Black));
    assert!(!term.attrs.flags.bold());
}

#[test]
fn test_multiple_sgr_attributes() {
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[1;3;4;9mTest");

    assert!(term.attrs.flags.bold());
    assert!(term.attrs.flags.italic());
    assert!(term.attrs.flags.underline());
    assert!(term.attrs.flags.strikethrough());
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
fn test_sgr_21_is_a_double_underline() {
    use crate::cell::UnderlineStyle;
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b[21mA\x1b[24mB");
    let grid = term.active_grid();
    let cell = grid.get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::Double);
    let cell = grid.get(1, 0).unwrap();
    assert!(!cell.flags.underline());
    assert_eq!(cell.flags.underline_style, UnderlineStyle::None);
}

#[test]
fn test_underline_color_semicolon_form() {
    let mut term = Terminal::new(80, 24);
    // `58;5;7` and `58;2;r;g;b`: the arguments belong to 58, they are not
    // SGR 5 (blink), 7 (reverse), 2 (dim) or 0 (reset).
    term.process(b"\x1b[4;58;5;7mA\x1b[0;4;58;2;255;0;0mB\x1b[0;4;58:5:1mC");
    let grid = term.active_grid();

    let cell = grid.get(0, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.underline_color(), Some(Color::from_ansi_code(7)));
    assert!(!cell.flags.blink() && !cell.flags.reverse());

    let cell = grid.get(1, 0).unwrap();
    assert!(cell.flags.underline());
    assert_eq!(cell.underline_color(), Some(Color::Rgb(255, 0, 0)));
    assert!(!cell.flags.dim());

    // The colon form still works.
    let cell = grid.get(2, 0).unwrap();
    assert_eq!(cell.underline_color(), Some(Color::from_ansi_code(1)));
}

#[test]
fn test_colon_truecolor_with_and_without_the_color_space() {
    let mut term = Terminal::new(80, 24);
    // ITU T.416: `38:2:<color space>:r:g:b`, the color space usually empty.
    term.process(b"\x1b[38:2::10:20:30;48:2::40:50:60;4;58:2::70:80:90mA");
    // Without the color-space slot the components follow the 2 directly.
    term.process(b"\x1b[0;38:2:11:21:31;4;58:2:71:81:91mB");
    let grid = term.active_grid();

    let cell = grid.get(0, 0).unwrap();
    assert_eq!(cell.fg(), Color::Rgb(10, 20, 30));
    assert_eq!(cell.bg(), Color::Rgb(40, 50, 60));
    assert_eq!(cell.underline_color(), Some(Color::Rgb(70, 80, 90)));

    let cell = grid.get(1, 0).unwrap();
    assert_eq!(cell.fg(), Color::Rgb(11, 21, 31));
    assert_eq!(cell.underline_color(), Some(Color::Rgb(71, 81, 91)));
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
