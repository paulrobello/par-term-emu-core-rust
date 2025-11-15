//! ANSI sequence utilities for generation and parsing

use crate::color::Color;
use unicode_width::UnicodeWidthStr;

/// Strip all ANSI escape sequences from text
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Check for CSI sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Skip until we hit a letter (sequence terminator)
                while let Some(&next_c) = chars.peek() {
                    chars.next();
                    if next_c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            // Check for OSC sequence
            else if chars.peek() == Some(&']') {
                chars.next(); // consume ']'
                              // Skip until ST (ESC \ or BEL)
                while let Some(next_c) = chars.next() {
                    if next_c == '\x07' {
                        break; // BEL
                    }
                    if next_c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next(); // consume '\'
                        break; // ST
                    }
                }
            }
            // Other escape sequences
            else if let Some(&next_c) = chars.peek() {
                chars.next();
                if !next_c.is_ascii_alphabetic() {
                    // Two-byte sequence, skip one more
                    chars.next();
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Measure text width without ANSI codes (accounts for wide characters)
pub fn measure_text_width(text: &str) -> usize {
    strip_ansi(text).width()
}

/// Generate SGR (Select Graphic Rendition) sequence
#[allow(clippy::too_many_arguments)]
pub fn generate_sgr(
    reset: bool,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    reverse: bool,
    hidden: bool,
    strikethrough: bool,
    fg: Option<Color>,
    bg: Option<Color>,
) -> String {
    let mut codes = Vec::new();

    if reset {
        codes.push("0".to_string());
    }

    if bold {
        codes.push("1".to_string());
    }
    if dim {
        codes.push("2".to_string());
    }
    if italic {
        codes.push("3".to_string());
    }
    if underline {
        codes.push("4".to_string());
    }
    if blink {
        codes.push("5".to_string());
    }
    if reverse {
        codes.push("7".to_string());
    }
    if hidden {
        codes.push("8".to_string());
    }
    if strikethrough {
        codes.push("9".to_string());
    }

    if let Some(color) = fg {
        match color {
            Color::Named(c) => {
                codes.push(format!("{}", 30 + c as u8));
            }
            Color::Indexed(idx) => {
                codes.push(format!("38;5;{}", idx));
            }
            Color::Rgb(r, g, b) => {
                codes.push(format!("38;2;{};{};{}", r, g, b));
            }
        }
    }

    if let Some(color) = bg {
        match color {
            Color::Named(c) => {
                codes.push(format!("{}", 40 + c as u8));
            }
            Color::Indexed(idx) => {
                codes.push(format!("48;5;{}", idx));
            }
            Color::Rgb(r, g, b) => {
                codes.push(format!("48;2;{};{};{}", r, g, b));
            }
        }
    }

    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// Generate cursor positioning sequence
pub fn generate_cursor_move(row: usize, col: usize) -> String {
    format!("\x1b[{};{}H", row + 1, col + 1)
}

/// Generate clear screen sequence
pub fn generate_clear_screen() -> String {
    "\x1b[2J".to_string()
}

/// Generate clear line sequence
pub fn generate_clear_line() -> String {
    "\x1b[2K".to_string()
}

/// Parse color from string (hex, rgb, name)
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();

    // Hex color: #RRGGBB
    if s.starts_with('#') && s.len() == 7 {
        let r = u8::from_str_radix(&s[1..3], 16).ok()?;
        let g = u8::from_str_radix(&s[3..5], 16).ok()?;
        let b = u8::from_str_radix(&s[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }

    // RGB function: rgb(r, g, b)
    if s.starts_with("rgb(") && s.ends_with(')') {
        let inner = &s[4..s.len() - 1];
        let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
        if parts.len() == 3 {
            let r = parts[0].parse::<u8>().ok()?;
            let g = parts[1].parse::<u8>().ok()?;
            let b = parts[2].parse::<u8>().ok()?;
            return Some(Color::Rgb(r, g, b));
        }
    }

    // Named colors
    use crate::color::NamedColor;
    let color = match s.to_lowercase().as_str() {
        "black" => NamedColor::Black,
        "red" => NamedColor::Red,
        "green" => NamedColor::Green,
        "yellow" => NamedColor::Yellow,
        "blue" => NamedColor::Blue,
        "magenta" => NamedColor::Magenta,
        "cyan" => NamedColor::Cyan,
        "white" => NamedColor::White,
        "bright_black" | "gray" | "grey" => NamedColor::BrightBlack,
        "bright_red" => NamedColor::BrightRed,
        "bright_green" => NamedColor::BrightGreen,
        "bright_yellow" => NamedColor::BrightYellow,
        "bright_blue" => NamedColor::BrightBlue,
        "bright_magenta" => NamedColor::BrightMagenta,
        "bright_cyan" => NamedColor::BrightCyan,
        "bright_white" => NamedColor::BrightWhite,
        _ => return None,
    };

    Some(Color::Named(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("hello world"), "hello world");
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[1;32mbold green\x1b[0m"), "bold green");
        assert_eq!(
            strip_ansi("normal \x1b[31mred\x1b[0m normal"),
            "normal red normal"
        );
    }

    #[test]
    fn test_measure_text_width() {
        assert_eq!(measure_text_width("hello"), 5);
        assert_eq!(measure_text_width("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(measure_text_width("hello 世界"), 10); // 5 + 1 + 2*2 = 10
    }

    #[test]
    fn test_generate_sgr() {
        let seq = generate_sgr(
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(Color::Rgb(255, 0, 0)),
            None,
        );
        assert_eq!(seq, "\x1b[1;38;2;255;0;0m");
    }

    #[test]
    fn test_generate_cursor_move() {
        assert_eq!(generate_cursor_move(0, 0), "\x1b[1;1H");
        assert_eq!(generate_cursor_move(5, 10), "\x1b[6;11H");
    }

    #[test]
    fn test_parse_color() {
        assert!(matches!(
            parse_color("#FF0000"),
            Some(Color::Rgb(255, 0, 0))
        ));
        assert!(matches!(
            parse_color("rgb(255, 0, 0)"),
            Some(Color::Rgb(255, 0, 0))
        ));
        assert!(matches!(parse_color("red"), Some(Color::Named(_))));
        assert!(parse_color("invalid").is_none());
    }
}
