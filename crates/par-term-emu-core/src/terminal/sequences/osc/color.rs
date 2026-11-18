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

/// Parse an OSC 4/104 palette index operand. Only the 16 ANSI slots are
/// addressable, so anything non-UTF-8, non-numeric, or >= 16 is `None`.
fn parse_palette_index(param: &[u8]) -> Option<usize> {
    let index = std::str::from_utf8(param)
        .ok()?
        .trim()
        .parse::<usize>()
        .ok()?;
    (index < 16).then_some(index)
}

/// The theme slot an OSC 10/11/12 dynamic-color command addresses.
/// Unknown commands map to `None`, so the compiler-visible mapping here is
/// the only place a new dynamic-color command needs to be registered.
fn dynamic_color_slot<'a>(theme: &'a mut ColorThemeState, command: &str) -> Option<&'a mut Color> {
    match command {
        "10" => Some(&mut theme.default_fg),
        "11" => Some(&mut theme.default_bg),
        "12" => Some(&mut theme.cursor_color),
        _ => None,
    }
}

/// OSC 10/11/12 with at least one operand: `?` queries the slot, anything
/// else sets it (unless insecure sequences are disabled).
fn handle_dynamic_color(
    theme: &mut ColorThemeState,
    response: &mut Vec<u8>,
    disable_insecure_sequences: bool,
    command: &str,
    params: &[&[u8]],
) {
    let Some(slot) = dynamic_color_slot(theme, command) else {
        return;
    };
    let Some(data) = params.get(1).and_then(|p| std::str::from_utf8(p).ok()) else {
        return;
    };
    let data = data.trim();
    if data == "?" {
        let (r, g, b) = slot.to_rgb();
        let r16 = (r as u16) * 257;
        let g16 = (g as u16) * 257;
        let b16 = (b as u16) * 257;
        let reply = format!(
            "\x1b]{};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
            command, r16, g16, b16
        );
        response.extend_from_slice(reply.as_bytes());
        return;
    }
    if disable_insecure_sequences {
        return;
    }
    if let Some((r, g, b)) = Terminal::parse_color_spec(data) {
        *slot = Color::Rgb(r, g, b);
    }
}

/// OSC 4 (set one ANSI palette entry): `4;index;colorspec`.
fn handle_palette_set(theme: &mut ColorThemeState, params: &[&[u8]]) {
    let Some(index) = parse_palette_index(params[1]) else {
        return;
    };
    let Ok(colorspec) = std::str::from_utf8(params[2]) else {
        return;
    };
    if let Some((r, g, b)) = Terminal::parse_color_spec(colorspec) {
        theme.ansi_palette[index] = Color::Rgb(r, g, b);
    }
}

/// OSC 104 (reset ANSI palette): no operand or an empty one resets all 16
/// entries, otherwise only the named index.
fn handle_palette_reset(theme: &mut ColorThemeState, params: &[&[u8]]) {
    if params.len() == 1 || (params.len() >= 2 && params[1].is_empty()) {
        theme.ansi_palette = Terminal::default_ansi_palette();
        return;
    }
    if let Some(index) = params.get(1).and_then(|p| parse_palette_index(p)) {
        theme.ansi_palette[index] = Terminal::default_ansi_palette()[index];
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
        "4" if !disable_insecure_sequences && params.len() >= 3 => {
            handle_palette_set(theme, params);
        }
        "104" if !disable_insecure_sequences => handle_palette_reset(theme, params),
        "10" | "11" | "12" if params.len() >= 2 => {
            handle_dynamic_color(theme, response, disable_insecure_sequences, command, params);
        }
        "110" if !disable_insecure_sequences => {
            theme.default_fg = Color::Rgb(0xE5, 0xE5, 0xE5);
        }
        "111" if !disable_insecure_sequences => {
            theme.default_bg = Color::Rgb(0x14, 0x19, 0x1E);
        }
        "112" if !disable_insecure_sequences => {
            theme.cursor_color = Color::Rgb(0xE5, 0xE5, 0xE5);
        }
        _ => {}
    }
}
