//! Color-related OSC sequence handling

use crate::color::Color;
use crate::terminal::{ColorThemeState, Terminal};

impl Terminal {
    /// Parse X11/xterm color specification to RGB tuple
    pub(crate) fn parse_color_spec(spec: &str) -> Option<(u8, u8, u8)> {
        let spec = spec.trim();

        if spec.is_empty() {
            return None;
        }

        // Format: rgb:RR/GG/BB (case-insensitive)
        if spec.to_lowercase().starts_with("rgb:") {
            let parts: Vec<&str> = spec[4..].split('/').collect();
            if parts.len() != 3 {
                return None;
            }

            // Parse hex components (1-4 hex digits each, we use first 2)
            let r = u8::from_str_radix(&format!("{:0<2}", &parts[0][..parts[0].len().min(2)]), 16)
                .ok()?;
            let g = u8::from_str_radix(&format!("{:0<2}", &parts[1][..parts[1].len().min(2)]), 16)
                .ok()?;
            let b = u8::from_str_radix(&format!("{:0<2}", &parts[2][..parts[2].len().min(2)]), 16)
                .ok()?;
            return Some((r, g, b));
        }

        // Format: #RRGGBB (case-insensitive)
        if spec.starts_with('#') && spec.len() == 7 {
            let r = u8::from_str_radix(&spec[1..3], 16).ok()?;
            let g = u8::from_str_radix(&spec[3..5], 16).ok()?;
            let b = u8::from_str_radix(&spec[5..7], 16).ok()?;
            return Some((r, g, b));
        }

        None
    }
}

/// OSC 4/104 (palette), OSC 10/11/12 (dynamic colors, query or set),
/// OSC 110/111/112 (reset dynamic colors).
///
/// Capability boundary (ARC-002): takes the color theme, the reply buffer,
/// and the insecure-sequence policy flag rather than `&mut Terminal`.
pub(crate) fn handle_osc_color(
    theme: &mut ColorThemeState,
    response: &mut Vec<u8>,
    disable_insecure_sequences: bool,
    command: &str,
    params: &[&[u8]],
) {
    match command {
            "4"
                // Set ANSI color palette entry (OSC 4)
                if !disable_insecure_sequences && params.len() >= 3 => {
                    if let Ok(data) = std::str::from_utf8(params[1]) {
                        if let Ok(index) = data.trim().parse::<usize>() {
                            if index < 16 {
                                if let Ok(colorspec) = std::str::from_utf8(params[2]) {
                                    if let Some((r, g, b)) = Terminal::parse_color_spec(colorspec) {
                                        theme.ansi_palette[index] = Color::Rgb(r, g, b);
                                    }
                                }
                            }
                        }
                    }
                }
            "104"
                // Reset ANSI color palette (OSC 104)
                if !disable_insecure_sequences => {
                    if params.len() == 1 || (params.len() >= 2 && params[1].is_empty()) {
                        theme.ansi_palette = Terminal::default_ansi_palette();
                    } else if params.len() >= 2 {
                        if let Ok(data) = std::str::from_utf8(params[1]) {
                            if let Ok(index) = data.trim().parse::<usize>() {
                                if index < 16 {
                                    let defaults = Terminal::default_ansi_palette();
                                    theme.ansi_palette[index] = defaults[index];
                                }
                            }
                        }
                    }
                }
            "10" | "11" | "12"
                // Query or set default colors
                if params.len() >= 2 => {
                    if let Ok(data) = std::str::from_utf8(params[1]) {
                        let data = data.trim();
                        if data == "?" {
                            let color = match command {
                                "10" => theme.default_fg,
                                "11" => theme.default_bg,
                                "12" => theme.cursor_color,
                                _ => unreachable!(),
                            };
                            let (r, g, b) = color.to_rgb();
                            let r16 = (r as u16) * 257;
                            let g16 = (g as u16) * 257;
                            let b16 = (b as u16) * 257;
                            let reply = format!(
                                "\x1b]{};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                                command, r16, g16, b16
                            );
                            response.extend_from_slice(reply.as_bytes());
                        } else if !disable_insecure_sequences {
                            if let Some((r, g, b)) = Terminal::parse_color_spec(data) {
                                match command {
                                    "10" => theme.default_fg = Color::Rgb(r, g, b),
                                    "11" => theme.default_bg = Color::Rgb(r, g, b),
                                    "12" => theme.cursor_color = Color::Rgb(r, g, b),
                                    _ => unreachable!(),
                                }
                            }
                        }
                    }
                }
            "110"
                if !disable_insecure_sequences => {
                    theme.default_fg = Color::Rgb(0xE5, 0xE5, 0xE5);
                }
            "111"
                if !disable_insecure_sequences => {
                    theme.default_bg = Color::Rgb(0x14, 0x19, 0x1E);
                }
            "112"
                if !disable_insecure_sequences => {
                    theme.cursor_color = Color::Rgb(0xE5, 0xE5, 0xE5);
                }
            _ => {}
        }
}
