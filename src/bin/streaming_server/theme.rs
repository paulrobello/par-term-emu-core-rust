//! Terminal color themes for the standalone streaming server (QA-013 split).

use par_term_emu_core_rust::color::Color;
use par_term_emu_core_rust::streaming::protocol::ThemeInfo;
use par_term_emu_core_rust::terminal::Terminal;

/// Terminal color theme definition
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub foreground: Color,
    pub normal: [Color; 8],
    pub bright: [Color; 8],
}

impl Theme {
    /// Create iTerm2 dark theme
    pub fn iterm2_dark() -> Self {
        Self {
            name: "iTerm2-dark".to_string(),
            background: Color::Rgb(0, 0, 0),
            foreground: Color::Rgb(255, 255, 255),
            normal: [
                Color::Rgb(0, 0, 0),
                Color::Rgb(201, 27, 0),
                Color::Rgb(0, 194, 0),
                Color::Rgb(199, 196, 0),
                Color::Rgb(2, 37, 199),
                Color::Rgb(201, 48, 199),
                Color::Rgb(0, 197, 199),
                Color::Rgb(199, 199, 199),
            ],
            bright: [
                Color::Rgb(104, 104, 104),
                Color::Rgb(255, 110, 103),
                Color::Rgb(95, 249, 103),
                Color::Rgb(254, 251, 103),
                Color::Rgb(104, 113, 255),
                Color::Rgb(255, 118, 255),
                Color::Rgb(96, 253, 255),
                Color::Rgb(255, 255, 255),
            ],
        }
    }

    /// Create Monokai theme
    pub fn monokai() -> Self {
        Self {
            name: "monokai".to_string(),
            background: Color::Rgb(12, 12, 12),
            foreground: Color::Rgb(217, 217, 217),
            normal: [
                Color::Rgb(26, 26, 26),
                Color::Rgb(244, 0, 95),
                Color::Rgb(152, 224, 36),
                Color::Rgb(253, 151, 31),
                Color::Rgb(157, 101, 255),
                Color::Rgb(244, 0, 95),
                Color::Rgb(88, 209, 235),
                Color::Rgb(196, 197, 181),
            ],
            bright: [
                Color::Rgb(98, 94, 76),
                Color::Rgb(244, 0, 95),
                Color::Rgb(152, 224, 36),
                Color::Rgb(224, 213, 97),
                Color::Rgb(157, 101, 255),
                Color::Rgb(244, 0, 95),
                Color::Rgb(88, 209, 235),
                Color::Rgb(246, 246, 239),
            ],
        }
    }

    /// Create Dracula theme
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            background: Color::Rgb(40, 42, 54),
            foreground: Color::Rgb(248, 248, 242),
            normal: [
                Color::Rgb(33, 34, 44),
                Color::Rgb(255, 85, 85),
                Color::Rgb(80, 250, 123),
                Color::Rgb(241, 250, 140),
                Color::Rgb(189, 147, 249),
                Color::Rgb(255, 121, 198),
                Color::Rgb(139, 233, 253),
                Color::Rgb(248, 248, 242),
            ],
            bright: [
                Color::Rgb(98, 114, 164),
                Color::Rgb(255, 110, 110),
                Color::Rgb(105, 255, 148),
                Color::Rgb(255, 255, 165),
                Color::Rgb(214, 172, 255),
                Color::Rgb(255, 146, 223),
                Color::Rgb(164, 255, 255),
                Color::Rgb(255, 255, 255),
            ],
        }
    }

    /// Create Solarized Dark theme
    pub fn solarized_dark() -> Self {
        Self {
            name: "solarized-dark".to_string(),
            background: Color::Rgb(0, 43, 54),
            foreground: Color::Rgb(131, 148, 150),
            normal: [
                Color::Rgb(7, 54, 66),
                Color::Rgb(220, 50, 47),
                Color::Rgb(133, 153, 0),
                Color::Rgb(181, 137, 0),
                Color::Rgb(38, 139, 210),
                Color::Rgb(211, 54, 130),
                Color::Rgb(42, 161, 152),
                Color::Rgb(238, 232, 213),
            ],
            bright: [
                Color::Rgb(0, 43, 54),
                Color::Rgb(203, 75, 22),
                Color::Rgb(88, 110, 117),
                Color::Rgb(101, 123, 131),
                Color::Rgb(131, 148, 150),
                Color::Rgb(108, 113, 196),
                Color::Rgb(147, 161, 161),
                Color::Rgb(253, 246, 227),
            ],
        }
    }

    /// Get theme by name
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "iterm2-dark" => Some(Self::iterm2_dark()),
            "monokai" => Some(Self::monokai()),
            "dracula" => Some(Self::dracula()),
            "solarized-dark" => Some(Self::solarized_dark()),
            _ => None,
        }
    }

    /// Get list of available theme names
    pub fn available() -> Vec<&'static str> {
        vec!["iterm2-dark", "monokai", "dracula", "solarized-dark"]
    }

    /// Apply theme to terminal
    pub fn apply(&self, terminal: &mut Terminal) {
        terminal.set_default_bg(self.background);
        terminal.set_default_fg(self.foreground);

        // Set normal colors (0-7)
        for (i, color) in self.normal.iter().enumerate() {
            let _ = terminal.set_ansi_palette_color(i, *color);
        }

        // Set bright colors (8-15)
        for (i, color) in self.bright.iter().enumerate() {
            let _ = terminal.set_ansi_palette_color(i + 8, *color);
        }
    }

    /// Convert theme to protocol ThemeInfo for sending to clients
    pub fn to_protocol(&self) -> ThemeInfo {
        ThemeInfo {
            name: self.name.clone(),
            background: self.background.to_rgb(),
            foreground: self.foreground.to_rgb(),
            normal: [
                self.normal[0].to_rgb(),
                self.normal[1].to_rgb(),
                self.normal[2].to_rgb(),
                self.normal[3].to_rgb(),
                self.normal[4].to_rgb(),
                self.normal[5].to_rgb(),
                self.normal[6].to_rgb(),
                self.normal[7].to_rgb(),
            ],
            bright: [
                self.bright[0].to_rgb(),
                self.bright[1].to_rgb(),
                self.bright[2].to_rgb(),
                self.bright[3].to_rgb(),
                self.bright[4].to_rgb(),
                self.bright[5].to_rgb(),
                self.bright[6].to_rgb(),
                self.bright[7].to_rgb(),
            ],
        }
    }
}
