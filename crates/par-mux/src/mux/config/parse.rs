//! The shared config spellings: the crate's one prefix grammar and the
//! `#rrggbb` hex colors.

/// Parse a `#rrggbb` hex color (the border-color config spellings).
/// `None` for anything else — callers keep their built-in default.
#[must_use]
pub fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

/// Parse the tmux prefix spelling (`C-b`, `C-a`, `C-Space`) or a literal
/// single character into its byte. The crate's ONE prefix grammar: the
/// attach client's `--prefix` and every chord in the config file are
/// parsed through this, so the spellings cannot drift apart.
pub fn parse_prefix(spec: &str) -> Option<u8> {
    let lower = spec.to_ascii_lowercase();
    if let Some(key) = lower.strip_prefix("c-") {
        return match key {
            "space" => Some(0x00),
            letter if letter.len() == 1 && letter.as_bytes()[0].is_ascii_lowercase() => {
                Some(letter.as_bytes()[0] - b'a' + 1)
            }
            _ => None,
        };
    }
    let bytes = spec.as_bytes();
    if bytes.len() == 1 {
        Some(bytes[0])
    } else {
        None
    }
}
