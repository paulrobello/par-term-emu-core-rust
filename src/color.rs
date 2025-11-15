/// Color representation supporting various color modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    /// Named ANSI colors (0-15)
    Named(NamedColor),
    /// 256-color palette
    Indexed(u8),
    /// 24-bit RGB color
    Rgb(u8, u8, u8),
}

/// Named ANSI colors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NamedColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

impl Default for Color {
    fn default() -> Self {
        Color::Named(NamedColor::White)
    }
}

impl Color {
    /// Convert color to RGB values
    pub fn to_rgb(&self) -> (u8, u8, u8) {
        match self {
            Color::Named(named) => named.to_rgb(),
            Color::Indexed(idx) => indexed_to_rgb(*idx),
            Color::Rgb(r, g, b) => (*r, *g, *b),
        }
    }

    /// Create color from ANSI color code
    pub fn from_ansi_code(code: u8) -> Self {
        match code {
            0..=15 => Color::Named(NamedColor::from_u8(code)),
            _ => Color::Indexed(code),
        }
    }
}

impl NamedColor {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => NamedColor::Black,
            1 => NamedColor::Red,
            2 => NamedColor::Green,
            3 => NamedColor::Yellow,
            4 => NamedColor::Blue,
            5 => NamedColor::Magenta,
            6 => NamedColor::Cyan,
            7 => NamedColor::White,
            8 => NamedColor::BrightBlack,
            9 => NamedColor::BrightRed,
            10 => NamedColor::BrightGreen,
            11 => NamedColor::BrightYellow,
            12 => NamedColor::BrightBlue,
            13 => NamedColor::BrightMagenta,
            14 => NamedColor::BrightCyan,
            _ => NamedColor::BrightWhite,
        }
    }

    fn to_rgb(self) -> (u8, u8, u8) {
        match self {
            NamedColor::Black => (0, 0, 0),
            NamedColor::Red => (128, 0, 0),
            NamedColor::Green => (0, 128, 0),
            NamedColor::Yellow => (128, 128, 0),
            NamedColor::Blue => (0, 0, 128),
            NamedColor::Magenta => (128, 0, 128),
            NamedColor::Cyan => (0, 128, 128),
            NamedColor::White => (192, 192, 192),
            NamedColor::BrightBlack => (128, 128, 128),
            NamedColor::BrightRed => (255, 0, 0),
            NamedColor::BrightGreen => (0, 255, 0),
            NamedColor::BrightYellow => (255, 255, 0),
            NamedColor::BrightBlue => (0, 0, 255),
            NamedColor::BrightMagenta => (255, 0, 255),
            NamedColor::BrightCyan => (0, 255, 255),
            NamedColor::BrightWhite => (255, 255, 255),
        }
    }
}

/// Convert 256-color index to RGB
fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        // Standard colors (0-15)
        0..=15 => NamedColor::from_u8(idx).to_rgb(),
        // 216 color cube (16-231)
        16..=231 => {
            let idx = idx - 16;
            let r = (idx / 36) * 51;
            let g = ((idx % 36) / 6) * 51;
            let b = (idx % 6) * 51;
            (r, g, b)
        }
        // Grayscale (232-255)
        232..=255 => {
            let gray = 8 + (idx - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_named_colors() {
        let black = Color::Named(NamedColor::Black);
        assert_eq!(black.to_rgb(), (0, 0, 0));

        let red = Color::Named(NamedColor::Red);
        assert_eq!(red.to_rgb(), (128, 0, 0));
    }

    #[test]
    fn test_rgb_color() {
        let color = Color::Rgb(255, 128, 64);
        assert_eq!(color.to_rgb(), (255, 128, 64));
    }

    #[test]
    fn test_indexed_color() {
        let color = Color::Indexed(196); // Bright red in 256 color palette
        let (r, _g, _b) = color.to_rgb();
        assert!(r > 200);
    }

    #[test]
    fn test_all_named_colors() {
        let colors = [
            (NamedColor::Black, (0, 0, 0)),
            (NamedColor::Red, (128, 0, 0)),
            (NamedColor::Green, (0, 128, 0)),
            (NamedColor::Yellow, (128, 128, 0)),
            (NamedColor::Blue, (0, 0, 128)),
            (NamedColor::Magenta, (128, 0, 128)),
            (NamedColor::Cyan, (0, 128, 128)),
            (NamedColor::White, (192, 192, 192)),
            (NamedColor::BrightBlack, (128, 128, 128)),
            (NamedColor::BrightRed, (255, 0, 0)),
            (NamedColor::BrightGreen, (0, 255, 0)),
            (NamedColor::BrightYellow, (255, 255, 0)),
            (NamedColor::BrightBlue, (0, 0, 255)),
            (NamedColor::BrightMagenta, (255, 0, 255)),
            (NamedColor::BrightCyan, (0, 255, 255)),
            (NamedColor::BrightWhite, (255, 255, 255)),
        ];

        for (color, expected) in &colors {
            let rgb = Color::Named(*color).to_rgb();
            assert_eq!(rgb, *expected);
        }
    }

    #[test]
    fn test_from_ansi_code() {
        // Test named colors (0-15)
        assert_eq!(Color::from_ansi_code(0), Color::Named(NamedColor::Black));
        assert_eq!(
            Color::from_ansi_code(15),
            Color::Named(NamedColor::BrightWhite)
        );

        // Test indexed colors (16-255)
        assert_eq!(Color::from_ansi_code(16), Color::Indexed(16));
        assert_eq!(Color::from_ansi_code(255), Color::Indexed(255));
    }

    #[test]
    fn test_color_cube_256() {
        // Test a few points in the 216 color cube (16-231)
        let color = Color::Indexed(16); // First color in cube (0,0,0)
        assert_eq!(color.to_rgb(), (0, 0, 0));

        let color = Color::Indexed(231); // Last color in cube (5,5,5) -> (255,255,255)
        assert_eq!(color.to_rgb(), (255, 255, 255));

        // Test a mid-range color
        let color = Color::Indexed(123);
        let rgb = color.to_rgb();
        // Color 123 = 16 + 107, where 107 = 2*36 + 5*6 + 5 = (2,5,5) -> (102,255,255)
        assert_eq!(rgb, (102, 255, 255));

        // Test a color with all non-zero components: 146 = 16 + 130, 130 = 3*36 + 3*6 + 4 = (3,3,4)
        let color = Color::Indexed(146);
        let (r, g, b) = color.to_rgb();
        assert!(r > 0 && r < 255);
        assert!(g > 0 && g < 255);
        assert!(b > 0 && b < 255);
    }

    #[test]
    fn test_grayscale_256() {
        // Test grayscale ramp (232-255)
        let color = Color::Indexed(232); // Darkest gray
        let (r, g, b) = color.to_rgb();
        assert_eq!(r, g);
        assert_eq!(g, b);
        assert_eq!(r, 8);

        let color = Color::Indexed(255); // Lightest gray
        let (r, g, b) = color.to_rgb();
        assert_eq!(r, g);
        assert_eq!(g, b);
        assert_eq!(r, 238);
    }

    #[test]
    fn test_color_default() {
        let color = Color::default();
        assert_eq!(color, Color::Named(NamedColor::White));
    }

    #[test]
    fn test_named_color_from_u8() {
        assert_eq!(NamedColor::from_u8(0), NamedColor::Black);
        assert_eq!(NamedColor::from_u8(7), NamedColor::White);
        assert_eq!(NamedColor::from_u8(15), NamedColor::BrightWhite);
        assert_eq!(NamedColor::from_u8(255), NamedColor::BrightWhite); // Out of range defaults to BrightWhite
    }
}
