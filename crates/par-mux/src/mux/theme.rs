//! Render-mode color themes (config `[client] theme`, `theme-dark`,
//! `theme-light`): herdr's built-in presets resolved to the handful of
//! colors the attach chrome paints with, plus the `auto` light/dark pick.
//!
//! Preset palettes ported from herdr (<https://github.com/ogulcancelik/herdr>,
//! Apache License 2.0) — `src/app/state.rs` `Palette::*` and
//! `src/config/theme.rs` (names and aliases), and the luminance rule from
//! `src/terminal_theme.rs` `RgbColor::inferred_appearance`. Ported
//! 2026-10-19. Only the tokens par-mux paints are kept; herdr's
//! `terminal` preset (16-color named palette) is not ported — the name
//! reads as `default`, which already themes through the host's indexed
//! colors.
//!
//! `default` resolves to no palette: every chrome site keeps its
//! built-in look (indexed accent, reversed status row, the measured
//! herdr tab-bar constants), so an absent key changes nothing.

/// An 8-bit-per-channel color.
pub type Rgb = (u8, u8, u8);

/// The colors a preset supplies to the attach chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Focused borders, the active tab block, overlay rings.
    pub accent: Rgb,
    /// The tab bar, and the text on accent-filled blocks.
    pub panel_bg: Rgb,
    /// The side panel's active-entry band.
    pub active_row_bg: Rgb,
    /// Inactive tab blocks and the status row background.
    pub surface0: Rgb,
    /// Muted text and unfocused borders/dividers.
    pub overlay0: Rgb,
    /// Main text.
    pub text: Rgb,
    /// Idle agents.
    pub green: Rgb,
    /// Working agents.
    pub yellow: Rgb,
    /// Blocked agents.
    pub red: Rgb,
    /// Done agents.
    pub teal: Rgb,
}

/// The built-in look (no palette).
pub const DEFAULT: &str = "default";
/// Follow the host's light/dark appearance.
pub const AUTO: &str = "auto";
/// `theme-light`'s default (herdr's light default).
pub const DEFAULT_LIGHT: &str = "catppuccin-latte";

/// Every `theme` spelling the settings dialog cycles: the built-in,
/// `auto`, then the presets in herdr's order.
pub const THEME_NAMES: &[&str] = &[
    DEFAULT,
    AUTO,
    "catppuccin",
    "catppuccin-latte",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "one-dark",
    "one-light",
    "solarized",
    "solarized-light",
    "kanagawa",
    "kanagawa-lotus",
    "rose-pine",
    "rose-pine-dawn",
    "vesper",
];

/// The `theme-dark` / `theme-light` choices: every name but `auto`.
pub const PRESET_NAMES: &[&str] = &[
    DEFAULT,
    "catppuccin",
    "catppuccin-latte",
    "tokyo-night",
    "tokyo-night-day",
    "dracula",
    "nord",
    "gruvbox",
    "gruvbox-light",
    "one-dark",
    "one-light",
    "solarized",
    "solarized-light",
    "kanagawa",
    "kanagawa-lotus",
    "rose-pine",
    "rose-pine-dawn",
    "vesper",
];

/// The canonical spelling of a theme name: case-insensitive, spaces and
/// underscores read as dashes, herdr's aliases accepted (`light` is
/// catppuccin-latte, herdr's `terminal` is `default`). `None` for an
/// unknown name.
#[must_use]
pub fn canonical(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().replace([' ', '_'], "-").as_str() {
        "" | "default" | "terminal" => Some(DEFAULT),
        "auto" => Some(AUTO),
        "catppuccin" | "catppuccin-mocha" => Some("catppuccin"),
        "catppuccin-latte" | "latte" | "light" => Some("catppuccin-latte"),
        "tokyo-night" | "tokyonight" => Some("tokyo-night"),
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => Some("tokyo-night-day"),
        "dracula" => Some("dracula"),
        "nord" => Some("nord"),
        "gruvbox" | "gruvbox-dark" => Some("gruvbox"),
        "gruvbox-light" => Some("gruvbox-light"),
        "one-dark" | "onedark" => Some("one-dark"),
        "one-light" | "onelight" => Some("one-light"),
        "solarized" | "solarized-dark" => Some("solarized"),
        "solarized-light" => Some("solarized-light"),
        "kanagawa" => Some("kanagawa"),
        "kanagawa-lotus" | "lotus" => Some("kanagawa-lotus"),
        "rose-pine" | "rosepine" => Some("rose-pine"),
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => Some("rose-pine-dawn"),
        "vesper" => Some("vesper"),
        _ => None,
    }
}

/// A preset's palette; `None` for `default`, `auto`, and unknown names
/// (the built-in look).
#[must_use]
pub fn palette(name: &str) -> Option<Palette> {
    let name = canonical(name)?;
    PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, palette)| *palette)
}

/// The host terminal's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// A dark background.
    Dark,
    /// A light background.
    Light,
}

/// herdr's light/dark rule over a background color: Rec. 601 luma
/// (299/587/114) at or above half scale reads as light.
#[must_use]
pub fn appearance_of((r, g, b): Rgb) -> Appearance {
    let luminance = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114;
    if luminance >= 128_000 {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

/// A client's theme choice plus what it knows about the host: the three
/// config spellings and the host appearance (from the OSC 11 background,
/// or an explicit mode-2031 `CSI ? 997 ; 1|2 n` report, which wins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSelection {
    /// Config `theme` as spelled.
    pub theme: String,
    /// Config `theme-dark` as spelled.
    pub dark: String,
    /// Config `theme-light` as spelled.
    pub light: String,
    /// The host appearance, once anything reported it.
    pub appearance: Option<Appearance>,
    /// Whether `appearance` came from an explicit mode-2031 report.
    pub appearance_explicit: bool,
}

impl Default for ThemeSelection {
    fn default() -> Self {
        Self {
            theme: DEFAULT.to_string(),
            dark: DEFAULT.to_string(),
            light: DEFAULT_LIGHT.to_string(),
            appearance: None,
            appearance_explicit: false,
        }
    }
}

impl ThemeSelection {
    /// The preset in force. `auto` picks `dark`/`light` by the host
    /// appearance, and an unknown host (no probe answer) counts as dark.
    /// Unknown names, and `auto` named as a light/dark choice, are the
    /// built-in look.
    #[must_use]
    pub fn effective(&self) -> &'static str {
        let theme = canonical(&self.theme).unwrap_or(DEFAULT);
        if theme != AUTO {
            return theme;
        }
        let pick = match self.appearance.unwrap_or(Appearance::Dark) {
            Appearance::Dark => &self.dark,
            Appearance::Light => &self.light,
        };
        match canonical(pick) {
            Some(AUTO) | None => DEFAULT,
            Some(name) => name,
        }
    }

    /// Record the host background's appearance unless an explicit report
    /// already decided it. Returns whether the appearance changed.
    pub fn observe_background(&mut self, bg: Rgb) -> bool {
        if self.appearance_explicit {
            return false;
        }
        let next = Some(appearance_of(bg));
        let changed = self.appearance != next;
        self.appearance = next;
        changed
    }

    /// Record an explicit appearance report (mode 2031). Returns whether
    /// the appearance changed.
    pub fn observe_report(&mut self, appearance: Appearance) -> bool {
        self.appearance_explicit = true;
        let changed = self.appearance != Some(appearance);
        self.appearance = Some(appearance);
        changed
    }
}

/// One warning per theme key that names no preset — what the launch
/// prints and a reload flashes; the renderer uses `default` for it.
#[must_use]
pub fn theme_warnings(chords: &crate::mux::config::Chords) -> Vec<String> {
    let mut warnings = Vec::new();
    if canonical(&chords.theme).is_none() {
        warnings.push(format!(
            "[client] theme {:?} unknown — using default (valid: {})",
            chords.theme,
            THEME_NAMES.join(", ")
        ));
    }
    for (key, value) in [
        ("theme-dark", &chords.theme_dark),
        ("theme-light", &chords.theme_light),
    ] {
        if !matches!(canonical(value), Some(name) if name != AUTO) {
            warnings.push(format!(
                "[client] {key} {value:?} unknown — using default (valid: {})",
                PRESET_NAMES.join(", ")
            ));
        }
    }
    warnings
}

/// A palette color as the renderer's color.
#[cfg(feature = "attach")]
pub(crate) fn rt((r, g, b): Rgb) -> ratatui::style::Color {
    ratatui::style::Color::Rgb(r, g, b)
}

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    (r, g, b)
}

/// herdr's presets, by canonical name.
const PRESETS: &[(&str, Palette)] = &[
    (
        "catppuccin",
        Palette {
            accent: rgb(137, 180, 250),
            panel_bg: rgb(24, 24, 37),
            active_row_bg: rgb(30, 30, 46),
            surface0: rgb(49, 50, 68),
            overlay0: rgb(108, 112, 134),
            text: rgb(205, 214, 244),
            green: rgb(166, 227, 161),
            yellow: rgb(249, 226, 175),
            red: rgb(243, 139, 168),
            teal: rgb(148, 226, 213),
        },
    ),
    (
        "catppuccin-latte",
        Palette {
            accent: rgb(30, 102, 245),
            panel_bg: rgb(239, 241, 245),
            active_row_bg: rgb(230, 233, 239),
            surface0: rgb(204, 208, 218),
            overlay0: rgb(156, 160, 176),
            text: rgb(76, 79, 105),
            green: rgb(64, 160, 43),
            yellow: rgb(223, 142, 29),
            red: rgb(210, 15, 57),
            teal: rgb(23, 146, 153),
        },
    ),
    (
        "tokyo-night",
        Palette {
            accent: rgb(122, 162, 247),
            panel_bg: rgb(26, 27, 38),
            active_row_bg: rgb(35, 38, 54),
            surface0: rgb(36, 40, 59),
            overlay0: rgb(86, 95, 137),
            text: rgb(192, 202, 245),
            green: rgb(158, 206, 106),
            yellow: rgb(224, 175, 104),
            red: rgb(247, 118, 142),
            teal: rgb(125, 207, 255),
        },
    ),
    (
        "tokyo-night-day",
        Palette {
            accent: rgb(46, 125, 233),
            panel_bg: rgb(225, 226, 231),
            active_row_bg: rgb(210, 211, 218),
            surface0: rgb(196, 200, 218),
            overlay0: rgb(137, 144, 179),
            text: rgb(55, 96, 191),
            green: rgb(88, 117, 57),
            yellow: rgb(140, 108, 62),
            red: rgb(245, 42, 101),
            teal: rgb(17, 140, 116),
        },
    ),
    (
        "dracula",
        Palette {
            accent: rgb(189, 147, 249),
            panel_bg: rgb(40, 42, 54),
            active_row_bg: rgb(55, 60, 82),
            surface0: rgb(68, 71, 90),
            overlay0: rgb(98, 114, 164),
            text: rgb(248, 248, 242),
            green: rgb(80, 250, 123),
            yellow: rgb(241, 250, 140),
            red: rgb(255, 85, 85),
            teal: rgb(139, 233, 253),
        },
    ),
    (
        "nord",
        Palette {
            accent: rgb(136, 192, 208),
            panel_bg: rgb(46, 52, 64),
            active_row_bg: rgb(67, 76, 94),
            surface0: rgb(59, 66, 82),
            overlay0: rgb(76, 86, 106),
            text: rgb(236, 239, 244),
            green: rgb(163, 190, 140),
            yellow: rgb(235, 203, 139),
            red: rgb(191, 97, 106),
            teal: rgb(143, 188, 187),
        },
    ),
    (
        "gruvbox",
        Palette {
            accent: rgb(215, 153, 33),
            panel_bg: rgb(40, 40, 40),
            active_row_bg: rgb(50, 49, 48),
            surface0: rgb(60, 56, 54),
            overlay0: rgb(146, 131, 116),
            text: rgb(235, 219, 178),
            green: rgb(184, 187, 38),
            yellow: rgb(250, 189, 47),
            red: rgb(251, 73, 52),
            teal: rgb(142, 192, 124),
        },
    ),
    (
        "gruvbox-light",
        Palette {
            accent: rgb(7, 102, 120),
            panel_bg: rgb(251, 241, 199),
            active_row_bg: rgb(242, 229, 188),
            surface0: rgb(235, 219, 178),
            overlay0: rgb(146, 131, 116),
            text: rgb(60, 56, 54),
            green: rgb(121, 116, 14),
            yellow: rgb(181, 118, 20),
            red: rgb(157, 0, 6),
            teal: rgb(66, 123, 88),
        },
    ),
    (
        "one-dark",
        Palette {
            accent: rgb(97, 175, 239),
            panel_bg: rgb(40, 44, 52),
            active_row_bg: rgb(49, 54, 64),
            surface0: rgb(44, 49, 58),
            overlay0: rgb(92, 99, 112),
            text: rgb(171, 178, 191),
            green: rgb(152, 195, 121),
            yellow: rgb(229, 192, 123),
            red: rgb(224, 108, 117),
            teal: rgb(86, 182, 194),
        },
    ),
    (
        "one-light",
        Palette {
            accent: rgb(64, 120, 242),
            panel_bg: rgb(250, 250, 250),
            active_row_bg: rgb(216, 219, 226),
            surface0: rgb(240, 240, 241),
            overlay0: rgb(160, 161, 167),
            text: rgb(56, 58, 66),
            green: rgb(80, 161, 79),
            yellow: rgb(193, 132, 1),
            red: rgb(228, 86, 73),
            teal: rgb(1, 132, 188),
        },
    ),
    (
        "solarized",
        Palette {
            accent: rgb(38, 139, 210),
            panel_bg: rgb(0, 43, 54),
            active_row_bg: rgb(22, 75, 87),
            surface0: rgb(7, 54, 66),
            overlay0: rgb(88, 110, 117),
            text: rgb(147, 161, 161),
            green: rgb(133, 153, 0),
            yellow: rgb(181, 137, 0),
            red: rgb(220, 50, 47),
            teal: rgb(42, 161, 152),
        },
    ),
    (
        "solarized-light",
        Palette {
            accent: rgb(38, 139, 210),
            panel_bg: rgb(253, 246, 227),
            active_row_bg: rgb(238, 232, 213),
            surface0: rgb(238, 232, 213),
            overlay0: rgb(147, 161, 161),
            text: rgb(101, 123, 131),
            green: rgb(133, 153, 0),
            yellow: rgb(181, 137, 0),
            red: rgb(220, 50, 47),
            teal: rgb(42, 161, 152),
        },
    ),
    (
        "kanagawa",
        Palette {
            accent: rgb(126, 156, 216),
            panel_bg: rgb(31, 31, 40),
            active_row_bg: rgb(54, 54, 70),
            surface0: rgb(42, 42, 55),
            overlay0: rgb(114, 113, 105),
            text: rgb(220, 215, 186),
            green: rgb(118, 148, 106),
            yellow: rgb(192, 163, 110),
            red: rgb(195, 64, 67),
            teal: rgb(127, 180, 202),
        },
    ),
    (
        "kanagawa-lotus",
        Palette {
            accent: rgb(77, 105, 155),
            panel_bg: rgb(242, 236, 188),
            active_row_bg: rgb(213, 206, 163),
            surface0: rgb(220, 213, 172),
            overlay0: rgb(160, 156, 172),
            text: rgb(84, 84, 100),
            green: rgb(111, 137, 78),
            yellow: rgb(119, 113, 63),
            red: rgb(200, 64, 83),
            teal: rgb(78, 140, 162),
        },
    ),
    (
        "rose-pine",
        Palette {
            accent: rgb(196, 167, 231),
            panel_bg: rgb(25, 23, 36),
            active_row_bg: rgb(38, 35, 58),
            surface0: rgb(31, 29, 46),
            overlay0: rgb(110, 106, 134),
            text: rgb(224, 222, 244),
            green: rgb(49, 116, 143),
            yellow: rgb(246, 193, 119),
            red: rgb(235, 111, 146),
            teal: rgb(156, 207, 216),
        },
    ),
    (
        "rose-pine-dawn",
        Palette {
            accent: rgb(144, 122, 169),
            panel_bg: rgb(250, 244, 237),
            active_row_bg: rgb(227, 217, 207),
            surface0: rgb(242, 233, 225),
            overlay0: rgb(152, 147, 165),
            text: rgb(70, 66, 97),
            green: rgb(40, 105, 131),
            yellow: rgb(234, 157, 52),
            red: rgb(180, 99, 122),
            teal: rgb(86, 148, 159),
        },
    ),
    (
        "vesper",
        Palette {
            accent: rgb(255, 199, 153),
            panel_bg: rgb(26, 26, 26),
            active_row_bg: rgb(16, 16, 16),
            surface0: rgb(35, 35, 35),
            overlay0: rgb(92, 92, 92),
            text: rgb(255, 255, 255),
            green: rgb(153, 255, 228),
            yellow: rgb(255, 199, 153),
            red: rgb(255, 128, 128),
            teal: rgb(102, 221, 204),
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every preset name has a palette, every listed name canonicalizes
    /// to itself, and the dialog lists match the preset table.
    #[test]
    fn theme_names_resolve_to_their_presets() {
        for name in THEME_NAMES {
            assert_eq!(canonical(name), Some(*name), "{name}");
        }
        let presets: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
        assert_eq!(&THEME_NAMES[2..], presets.as_slice());
        assert_eq!(&PRESET_NAMES[1..], presets.as_slice());
        assert_eq!(PRESET_NAMES[0], DEFAULT);
        assert!(palette(DEFAULT).is_none());
        assert!(palette(AUTO).is_none());
        assert!(palette("nope").is_none());
        assert_eq!(
            palette("Tokyo Night").map(|p| p.accent),
            Some((122, 162, 247)),
            "aliases and spacing resolve"
        );
        for alias in [
            "catppuccin-mocha",
            "tokyonight",
            "gruvbox-dark",
            "dawn",
            "light",
        ] {
            assert!(palette(alias).is_some(), "alias {alias}");
        }
        assert_eq!(canonical("terminal"), Some(DEFAULT), "herdr's 16-color");
        assert_eq!(canonical("dark"), None, "not a herdr alias");
    }

    /// Transcription guard: each light preset's panel background reads
    /// as light under herdr's luma rule, each dark one as dark.
    #[test]
    fn preset_panels_match_their_appearance() {
        let light = ["latte", "-day", "-light", "-dawn", "-lotus"];
        for (name, palette) in PRESETS {
            let want = if light.iter().any(|marker| name.contains(marker)) {
                Appearance::Light
            } else {
                Appearance::Dark
            };
            assert_eq!(appearance_of(palette.panel_bg), want, "{name}");
        }
    }

    /// `auto` follows the host: a light background picks `theme-light`,
    /// a dark one `theme-dark`, no answer counts as dark, and an explicit
    /// mode-2031 report beats a later background inference.
    #[test]
    fn auto_picks_from_the_host_appearance() {
        let mut sel = ThemeSelection {
            theme: "auto".into(),
            dark: "nord".into(),
            light: "one-light".into(),
            ..ThemeSelection::default()
        };
        assert_eq!(sel.effective(), "nord", "no probe answer = dark");
        assert!(sel.observe_background((0xff, 0xff, 0xff)));
        assert_eq!(sel.effective(), "one-light");
        assert!(sel.observe_background((0x1e, 0x1e, 0x2e)));
        assert_eq!(sel.effective(), "nord");
        assert!(sel.observe_report(Appearance::Light));
        assert!(!sel.observe_background((0, 0, 0)), "explicit report wins");
        assert_eq!(sel.effective(), "one-light");
        // A fixed theme ignores the host.
        sel.theme = "dracula".into();
        assert_eq!(sel.effective(), "dracula");
        // Unknown names and auto-as-a-pick fall back to the built-in.
        sel.theme = "bogus".into();
        assert_eq!(sel.effective(), DEFAULT);
        sel.theme = "auto".into();
        sel.light = "auto".into();
        assert_eq!(sel.effective(), DEFAULT);
        // The defaults: auto on a dark host is today's look, on a light
        // host herdr's latte.
        let mut sel = ThemeSelection {
            theme: "auto".into(),
            ..ThemeSelection::default()
        };
        assert_eq!(sel.effective(), DEFAULT);
        sel.observe_background((250, 250, 250));
        assert_eq!(sel.effective(), "catppuccin-latte");
    }

    #[test]
    fn theme_warnings_name_unknown_keys() {
        let mut chords = crate::mux::config::Chords::with_defaults();
        assert!(theme_warnings(&chords).is_empty());
        chords.theme = "Tokyo_Night".into();
        assert!(theme_warnings(&chords).is_empty(), "aliases are known");
        chords.theme = "catppucin".into();
        chords.theme_light = "auto".into();
        let warnings = theme_warnings(&chords);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("theme \"catppucin\""));
        assert!(warnings[1].contains("theme-light \"auto\""));
    }
}
