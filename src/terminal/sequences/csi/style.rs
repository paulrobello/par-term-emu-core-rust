//! SGR (Select Graphic Rendition) and style CSI sequence handling

use crate::cell::CellFlags;
use crate::color::{Color, NamedColor};
use crate::terminal::Terminal;
use vte::{Params, ParamsIter};

impl Terminal {
    pub(crate) fn handle_csi_style(&mut self, action: char, params: &Params, intermediates: &[u8]) {
        if action == 'm' {
            // Check for modifyOtherKeys mode setting: CSI > 4 ; mode m
            if intermediates.contains(&b'>') {
                let mut param_iter = params.iter();
                let first_param = param_iter
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);

                if first_param == 4 {
                    let mode = param_iter
                        .next()
                        .and_then(|p| p.first())
                        .copied()
                        .unwrap_or(0) as u8;
                    self.keyboard_state.modify_other_keys_mode = mode.min(2);
                    crate::debug_info!(
                        "CSI",
                        "modifyOtherKeys mode set to {}",
                        self.keyboard_state.modify_other_keys_mode
                    );
                }
                return;
            }

            // Check for modifyOtherKeys query: CSI ? 4 m
            if intermediates.contains(&b'?') {
                let param = params
                    .iter()
                    .next()
                    .and_then(|p| p.first())
                    .copied()
                    .unwrap_or(0);
                if param == 4 {
                    let response =
                        format!("\x1b[>4;{}m", self.keyboard_state.modify_other_keys_mode);
                    self.push_response(response.as_bytes());
                }
                return;
            }

            if params.is_empty() {
                self.flags = CellFlags::default();
                self.fg = self.theme.default_fg;
                self.bg = self.theme.default_bg;
                self.underline_color = None;
            } else {
                let mut iter = params.iter();
                while let Some(param_slice) = iter.next() {
                    let param = param_slice.first().copied().unwrap_or(0);
                    match param {
                        0 => {
                            self.flags = CellFlags::default();
                            self.fg = self.theme.default_fg;
                            self.bg = self.theme.default_bg;
                            self.underline_color = None;
                        }
                        1 => self.flags.set_bold(true),
                        2 => self.flags.set_dim(true),
                        3 => self.flags.set_italic(true),
                        4 => {
                            if let Some(&style_code) = param_slice.get(1) {
                                use crate::cell::UnderlineStyle;
                                self.flags.set_underline(true);
                                self.flags.underline_style = match style_code {
                                    0 => UnderlineStyle::None,
                                    1 => UnderlineStyle::Straight,
                                    2 => UnderlineStyle::Double,
                                    3 => UnderlineStyle::Curly,
                                    4 => UnderlineStyle::Dotted,
                                    5 => UnderlineStyle::Dashed,
                                    _ => UnderlineStyle::Straight,
                                };
                                if self.flags.underline_style == UnderlineStyle::None {
                                    self.flags.set_underline(false);
                                }
                            } else {
                                self.flags.set_underline(true);
                                self.flags.underline_style = crate::cell::UnderlineStyle::Straight;
                            }
                        }
                        5 => self.flags.set_blink(true),
                        7 => self.flags.set_reverse(true),
                        8 => self.flags.set_hidden(true),
                        9 => self.flags.set_strikethrough(true),
                        22 => {
                            self.flags.set_bold(false);
                            self.flags.set_dim(false);
                        }
                        23 => self.flags.set_italic(false),
                        24 => {
                            self.flags.set_underline(false);
                            self.flags.underline_style = crate::cell::UnderlineStyle::None;
                        }
                        25 => self.flags.set_blink(false),
                        27 => self.flags.set_reverse(false),
                        28 => self.flags.set_hidden(false),
                        29 => self.flags.set_strikethrough(false),
                        53 => self.flags.set_overline(true),
                        55 => self.flags.set_overline(false),
                        1004 => {
                            // Focus tracking (standard xterm extension in SGR? No, usually DECSET 1004)
                            // But some tests might use it in SGR. Let's add it to be safe if needed.
                            // Actually, standard xterm is DECSET 1004.
                        }
                        30..=37 => self.fg = Color::Named(NamedColor::from_u8((param - 30) as u8)),
                        38 => {
                            if let Some(color) = Self::parse_extended_color(param_slice, &mut iter)
                            {
                                self.fg = color;
                            }
                        }
                        39 => self.fg = self.theme.default_fg,
                        40..=47 => self.bg = Color::Named(NamedColor::from_u8((param - 40) as u8)),
                        48 => {
                            if let Some(color) = Self::parse_extended_color(param_slice, &mut iter)
                            {
                                self.bg = color;
                            }
                        }
                        49 => self.bg = self.theme.default_bg,
                        58 => {
                            // Set underline color (colon sub-parameter form only)
                            if let Some(color) = Self::parse_extended_color_subparams(param_slice) {
                                self.underline_color = Some(color);
                            }
                        }
                        59 => self.underline_color = None,
                        90..=97 => self.fg = Color::from_ansi_code((param - 90 + 8) as u8),
                        100..=107 => self.bg = Color::from_ansi_code((param - 100 + 8) as u8),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Parse an SGR extended-color specifier (`38`/`48`): either the colon
    /// sub-parameter form (`38:2:r:g:b` / `38:5:n`) carried entirely in
    /// `param_slice`, or the semicolon form (`38;2;r;g;b` / `38;5;n`) whose
    /// mode and value parameters are consumed from `iter`. Returns `None`
    /// when the parameters name no color; `iter` is only advanced when
    /// `param_slice` carries no sub-parameters.
    fn parse_extended_color(param_slice: &[u16], iter: &mut ParamsIter) -> Option<Color> {
        if param_slice.get(1).is_some() {
            Self::parse_extended_color_subparams(param_slice)
        } else if let Some(next) = iter.next() {
            match next.first().copied() {
                Some(2) => Some(Color::Rgb(
                    iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as u8,
                    iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as u8,
                    iter.next().and_then(|p| p.first()).copied().unwrap_or(0) as u8,
                )),
                Some(5) => iter
                    .next()
                    .and_then(|p| p.first())
                    .map(|&idx| Color::from_ansi_code(idx as u8)),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Parse the colon sub-parameter color form (`38:2:r:g:b` / `38:5:n`);
    /// missing RGB components default to 0.
    fn parse_extended_color_subparams(param_slice: &[u16]) -> Option<Color> {
        match param_slice.get(1).copied() {
            Some(2) => Some(Color::Rgb(
                param_slice.get(2).copied().unwrap_or(0) as u8,
                param_slice.get(3).copied().unwrap_or(0) as u8,
                param_slice.get(4).copied().unwrap_or(0) as u8,
            )),
            Some(5) => param_slice
                .get(2)
                .map(|&idx| Color::from_ansi_code(idx as u8)),
            _ => None,
        }
    }
}
