//! The shared config spellings: the crate's one prefix and chord
//! grammar and the `#rrggbb` hex colors.

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

/// One after-prefix chord key: the key byte and whether Alt (`M-`) is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordKey {
    /// The key byte (control keys are their C0 byte).
    pub key: u8,
    /// Alt/Meta is held (`M-x`).
    pub alt: bool,
}

impl ChordKey {
    /// A chord with no modifier.
    #[must_use]
    pub const fn plain(key: u8) -> Self {
        Self { key, alt: false }
    }

    /// A chord with Alt held.
    #[must_use]
    pub const fn alt(key: u8) -> Self {
        Self { key, alt: true }
    }
}

/// Parse a chord spelling: `M-x` (Alt + one key, case-insensitive prefix),
/// or anything [`parse_prefix`] accepts (`C-x`, a single character). The
/// combined `M-C-x` form is not supported.
pub fn parse_chord(spec: &str) -> Option<ChordKey> {
    let bytes = spec.as_bytes();
    if bytes.len() >= 2 && bytes[0].eq_ignore_ascii_case(&b'm') && bytes[1] == b'-' {
        let rest = &spec[2..];
        let rb = rest.as_bytes();
        return (rb.len() == 1).then(|| ChordKey::alt(rb[0]));
    }
    parse_prefix(spec).map(ChordKey::plain)
}

/// The inverse of [`parse_chord`]: `M-1`, `%`, `C-a`.
#[must_use]
pub fn spell_chord(c: &ChordKey) -> String {
    let plain = match c.key {
        0x00 => "C-Space".to_string(),
        k @ 0x01..=0x1a => format!("C-{}", char::from(k - 1 + b'a')),
        k => char::from(k).to_string(),
    };
    if c.alt {
        format!("M-{plain}")
    } else {
        plain
    }
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
