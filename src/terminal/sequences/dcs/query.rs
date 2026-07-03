//! XTGETTCAP (`DCS + q`) and DECRQSS (`DCS $ q`) query handling.
//!
//! Both share the `q` final byte with Sixel and are routed here by
//! `dcs_hook`/`dcs_unhook` in `mod.rs` based on their intermediate bytes.

use crate::color::{Color, NamedColor};
use crate::cursor::CursorStyle;
use crate::terminal::Terminal;

impl Terminal {
    /// XTGETTCAP reply: `DCS + q <hex-name>[;<hex-name>...] ST`.
    ///
    /// For each requested (hex-encoded) capability name, emits a separate
    /// reply: `DCS 1 + r <hexname>=<hexvalue> ST` if known, or
    /// `DCS 0 + r <hexname> ST` if unknown. xterm supports concatenating
    /// multiple results into one reply; emitting one DCS per name is
    /// simpler and equally valid, so that's what we do here.
    pub(crate) fn handle_xtgettcap_reply(&mut self) {
        let buffer = self.dcs_state.dcs_buffer.clone();
        let payload = String::from_utf8_lossy(&buffer);

        for hex_name in payload.split(';') {
            if hex_name.is_empty() {
                continue;
            }

            let value = decode_hex_to_string(hex_name).and_then(|name| lookup_capability(&name));

            let response = match value {
                Some(v) => format!("\x1bP1+r{}={}\x1b\\", hex_name, encode_hex(v.as_bytes())),
                None => format!("\x1bP0+r{}\x1b\\", hex_name),
            };
            self.push_response(response.as_bytes());
        }
    }

    /// DECRQSS reply: `DCS $ q <mnemonic> ST`.
    ///
    /// Replies `DCS 1 $ r <current-setting><mnemonic> ST` for recognized
    /// mnemonics (`m` SGR, ` q` DECSCUSR, `r` DECSTBM), or
    /// `DCS 0 $ r ST` for anything else.
    pub(crate) fn handle_decrqss_reply(&mut self) {
        let buffer = self.dcs_state.dcs_buffer.clone();
        let query = String::from_utf8_lossy(&buffer);

        let body = match query.as_ref() {
            "m" => Some(self.decrqss_sgr_body()),
            " q" => Some(format!("{} q", self.decrqss_cursor_style_number())),
            "r" => Some(format!(
                "{};{}r",
                self.margins.scroll_region_top + 1,
                self.margins.scroll_region_bottom + 1
            )),
            _ => None,
        };

        match body {
            Some(b) => {
                let response = format!("\x1bP1$r{}\x1b\\", b);
                self.push_response(response.as_bytes());
            }
            None => self.push_response(b"\x1bP0$r\x1b\\"),
        }
    }

    /// Build the SGR parameter string reflecting current pen state, e.g.
    /// `"0;1;4m"` for bold+underline, or `"0m"` for no attributes.
    fn decrqss_sgr_body(&self) -> String {
        let mut codes: Vec<String> = Vec::new();

        if self.flags.bold() {
            codes.push("1".to_string());
        }
        if self.flags.dim() {
            codes.push("2".to_string());
        }
        if self.flags.italic() {
            codes.push("3".to_string());
        }
        if self.flags.underline() {
            codes.push("4".to_string());
        }
        if self.flags.blink() {
            codes.push("5".to_string());
        }
        if self.flags.reverse() {
            codes.push("7".to_string());
        }
        if self.flags.hidden() {
            codes.push("8".to_string());
        }
        if self.flags.strikethrough() {
            codes.push("9".to_string());
        }

        // Only emit fg/bg codes when they differ from the terminal's
        // built-in defaults (Named White / Named Black) - matches how
        // xterm omits 39/49 for an untouched pen.
        if self.fg != Color::Named(NamedColor::White) {
            codes.push(sgr_color_code(self.fg, true));
        }
        if self.bg != Color::Named(NamedColor::Black) {
            codes.push(sgr_color_code(self.bg, false));
        }

        if codes.is_empty() {
            "0m".to_string()
        } else {
            format!("0;{}m", codes.join(";"))
        }
    }

    /// Current DECSCUSR cursor style number (1-6), per the same mapping
    /// used by the DECSCUSR setter in `sequences/csi/cursor.rs`.
    fn decrqss_cursor_style_number(&self) -> u8 {
        match self.cursor.style {
            CursorStyle::BlinkingBlock => 1,
            CursorStyle::SteadyBlock => 2,
            CursorStyle::BlinkingUnderline => 3,
            CursorStyle::SteadyUnderline => 4,
            CursorStyle::BlinkingBar => 5,
            CursorStyle::SteadyBar => 6,
        }
    }
}

/// SGR color parameter for a foreground (`is_fg = true`) or background color.
fn sgr_color_code(color: Color, is_fg: bool) -> String {
    match color {
        Color::Named(named) => {
            let n = named as u8;
            if n < 8 {
                format!("{}", if is_fg { 30 + n } else { 40 + n })
            } else {
                format!("{}", if is_fg { 90 + (n - 8) } else { 100 + (n - 8) })
            }
        }
        Color::Indexed(idx) => {
            format!("{};5;{}", if is_fg { 38 } else { 48 }, idx)
        }
        Color::Rgb(r, g, b) => {
            format!("{};2;{};{};{}", if is_fg { 38 } else { 48 }, r, g, b)
        }
    }
}

/// Curated termcap/terminfo capability table for XTGETTCAP.
fn lookup_capability(name: &str) -> Option<&'static str> {
    match name {
        "TN" | "name" => Some("xterm-256color"),
        "Co" | "colors" => Some("256"),
        // RGB: truecolor support, xterm replies with bits-per-channel (8)
        "RGB" => Some("8"),
        // Tc: truecolor flag capability (boolean - no meaningful value,
        // reported as known/present with an empty value)
        "Tc" => Some(""),
        _ => None,
    }
}

/// Decode a hex-encoded capability name (e.g. `"544e"` -> `"TN"`).
fn decode_hex_to_string(hex: &str) -> Option<String> {
    decode_hex_bytes(hex).and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Decode a hex string into raw bytes. Returns `None` on malformed input
/// (odd length or non-hex characters).
fn decode_hex_bytes(hex: &str) -> Option<Vec<u8>> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

/// Encode raw bytes as lowercase hex (e.g. `b"TN"` -> `"544e"`).
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
