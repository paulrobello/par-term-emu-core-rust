//! Key-event → terminal input byte encoding.
//!
//! One encoder for every frontend (par-term's winit app, ParDeck's iOS
//! renderer via the C FFI) so key translation is implemented once, in the
//! emulator, and respects the terminal's own negotiated state (application
//! cursor keys, kitty keyboard progressive-enhancement flags).
//!
//! Two regimes, selected by the terminal's `keyboard_flags`:
//!
//! - Legacy (flags 0, what a plain shell expects): xterm sequences —
//!   `ESC [ A` arrows, `ESC O A` under application cursor keys, control
//!   bytes for Ctrl+letters, `ESC` prefix for Alt.
//! - Kitty disambiguate mode (flags & 1): `CSI unicode-codepoint;mods u`
//!   for keys with Ctrl/Alt and for Enter/Tab/Backspace/Esc, per the kitty
//!   keyboard protocol level 1 (text keys without Ctrl/Alt stay plain text
//!   so typing is unaffected).
//!
//! Modifier bits follow the kitty protocol order: Shift=1, Alt=2, Ctrl=4,
//! Super=8, Hyper=16, Meta=32.

use crate::terminal::Terminal;

/// Modifier bitfield — `TermKeyEvent::modifiers`.
pub mod modifiers {
    pub const SHIFT: u8 = 1;
    pub const ALT: u8 = 2;
    pub const CTRL: u8 = 4;
    pub const SUPER: u8 = 8;
    pub const HYPER: u8 = 16;
    pub const META: u8 = 32;
}

/// The key pressed. Discriminants double as the kitty protocol functional
/// key codes (`57426` = Insert … `57376..57387` = F1..F12), so the kitty
/// encoder can cast straight through; do not renumber.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKey {
    Unknown = 0,
    /// `codepoint` carries the character
    Char = 1,
    Escape = 27,
    Tab = 9,
    Enter = 13,
    Backspace = 127,
    Insert = 57426,
    Delete = 57427,
    Left = 57428,
    Right = 57429,
    Up = 57430,
    Down = 57431,
    PageUp = 57432,
    PageDown = 57433,
    Home = 57434,
    End = 57435,
    F1 = 57376,
    F2 = 57377,
    F3 = 57378,
    F4 = 57379,
    F5 = 57380,
    F6 = 57381,
    F7 = 57382,
    F8 = 57383,
    F9 = 57384,
    F10 = 57385,
    F11 = 57386,
    F12 = 57387,
}

/// Map a raw wire/header value to a variant; anything that is not a
/// defined discriminant becomes `Unknown` (QA-151). The macro lists the
/// variants once so the match arms stay in lockstep with the enum.
macro_rules! term_key_from_raw {
    ($($variant:ident),* $(,)?) => {
        impl TermKey {
            pub fn from_raw(v: u16) -> TermKey {
                match v {
                    $(x if x == TermKey::$variant as u16 => TermKey::$variant,)*
                    _ => TermKey::Unknown,
                }
            }
        }
    };
}
term_key_from_raw!(
    Unknown, Char, Escape, Tab, Enter, Backspace, Insert, Delete, Left, Right, Up, Down, PageUp,
    PageDown, Home, End, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
);

/// A key event in a C-compatible layout.
///
/// For `TermKey::Char`, `codepoint` is the character as it should be typed:
/// the shifted glyph for plain typing (`'A'` for Shift+A, `'ä'` for ä), or
/// the base form (`'a'`) when Ctrl/Alt modifiers drive the encoding.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermKeyEvent {
    /// The key as a raw `TERM_KEY_*` value: C and Swift fill this struct
    /// with a bare `uint16_t`, and a Rust enum reference is only sound for
    /// a valid discriminant (QA-151) — every read goes through
    /// [`TermKeyEvent::key`], which maps unknown values to `Unknown`.
    pub key: u16,
    /// Bitfield of [`modifiers`] constants.
    pub modifiers: u8,
    /// Reserved; must be zero.
    pub _pad: u8,
    /// Unicode scalar for `TermKey::Char`, 0 otherwise.
    pub codepoint: u32,
}

impl TermKeyEvent {
    /// Build a `Char` event from a character and modifier bits.
    pub fn char_(c: char, mods: u8) -> Self {
        Self {
            key: TermKey::Char as u16,
            modifiers: mods,
            _pad: 0,
            codepoint: c as u32,
        }
    }

    /// Build a functional-key event.
    pub fn functional(key: TermKey, mods: u8) -> Self {
        Self {
            key: key as u16,
            modifiers: mods,
            _pad: 0,
            codepoint: 0,
        }
    }

    /// The validated key. The FFI fills `key` with a raw `uint16_t` from
    /// C/Swift; converting through here is what keeps an invalid
    /// discriminant from ever materializing as an enum value (QA-151).
    pub fn key(&self) -> TermKey {
        TermKey::from_raw(self.key)
    }

    fn ctrl(&self) -> bool {
        self.modifiers & modifiers::CTRL != 0
    }

    fn alt(&self) -> bool {
        self.modifiers & modifiers::ALT != 0
    }

    fn shift(&self) -> bool {
        self.modifiers & modifiers::SHIFT != 0
    }

    /// Only Shift (or nothing) — the "text-like" modifier set.
    fn text_like(&self) -> bool {
        self.modifiers & !(modifiers::SHIFT) == 0
    }

    /// xterm parameter form: 1 + set modifier bits.
    fn xterm_mod(&self) -> u8 {
        1 + (self.modifiers & 0x3f)
    }
}

/// F5..F12 `CSI n m ~` parameter numbers (F1..F4 use SS3 forms).
const F_TILDE: [(TermKey, u8); 8] = [
    (TermKey::F5, 15),
    (TermKey::F6, 17),
    (TermKey::F7, 18),
    (TermKey::F8, 19),
    (TermKey::F9, 20),
    (TermKey::F10, 21),
    (TermKey::F11, 23),
    (TermKey::F12, 24),
];

/// Final byte for the CSI/SS3 cursor-group keys.
fn cursor_group_final(key: TermKey) -> Option<u8> {
    match key {
        TermKey::Up => Some(b'A'),
        TermKey::Down => Some(b'B'),
        TermKey::Right => Some(b'C'),
        TermKey::Left => Some(b'D'),
        TermKey::Home => Some(b'H'),
        TermKey::End => Some(b'F'),
        _ => None,
    }
}

/// Legacy (xterm) encoding used when no kitty keyboard flags are set.
fn encode_legacy(ev: &TermKeyEvent, app_cursor: bool, out: &mut Vec<u8>) {
    let esc_prefix = |out: &mut Vec<u8>| {
        if ev.alt() {
            out.push(0x1b);
        }
    };

    match ev.key() {
        TermKey::Char => {
            let c = char::from_u32(ev.codepoint).unwrap_or('\0');
            if ev.text_like() {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            } else if ev.ctrl() {
                // Ctrl+letter/@.._ → control bytes; Ctrl+Space → NUL.
                let byte = match c.to_ascii_lowercase() {
                    'a'..='z' => (c.to_ascii_lowercase() as u8) - b'a' + 1,
                    '@'..='_' => c as u8 & 0x1f,
                    ' ' => 0,
                    _ => {
                        // Ctrl over a non-control character has no legacy
                        // form; send it through kitty-style CSI u below by
                        // falling back to the plain text.
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        return;
                    }
                };
                esc_prefix(out);
                out.push(byte);
            } else {
                // Alt (+Shift) text: ESC prefix + the character.
                esc_prefix(out);
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        TermKey::Escape => out.push(0x1b),
        TermKey::Enter => out.push(b'\r'),
        TermKey::Tab => {
            if ev.shift() {
                out.extend_from_slice(b"\x1b[Z");
            } else {
                out.push(b'\t');
            }
        }
        TermKey::Backspace => out.push(0x7f),
        TermKey::Insert | TermKey::Delete | TermKey::PageUp | TermKey::PageDown => {
            let n = match ev.key() {
                TermKey::Insert => 2,
                TermKey::Delete => 3,
                TermKey::PageUp => 5,
                _ => 6,
            };
            let m = ev.xterm_mod();
            if m == 1 {
                out.extend_from_slice(format!("\x1b[{}~", n).as_bytes());
            } else {
                out.extend_from_slice(format!("\x1b[{};{}~", n, m).as_bytes());
            }
        }
        TermKey::F1 | TermKey::F2 | TermKey::F3 | TermKey::F4 => {
            let final_byte = match ev.key() {
                TermKey::F1 => b'P',
                TermKey::F2 => b'Q',
                TermKey::F3 => b'R',
                _ => b'S',
            };
            let m = ev.xterm_mod();
            if m == 1 {
                out.push(0x1b);
                out.push(b'O');
                out.push(final_byte);
            } else {
                out.extend_from_slice(format!("\x1b[1;{}{}", m, final_byte as char).as_bytes());
            }
        }
        key => {
            if let Some(final_byte) = cursor_group_final(key) {
                let m = ev.xterm_mod();
                if m == 1 {
                    if app_cursor {
                        // DECCKM: SS3 form for unmodified cursor keys.
                        out.push(0x1b);
                        out.push(b'O');
                        out.push(final_byte);
                    } else {
                        out.push(0x1b);
                        out.push(b'[');
                        out.push(final_byte);
                    }
                } else {
                    out.extend_from_slice(format!("\x1b[1;{}{}", m, final_byte as char).as_bytes());
                }
            } else if let Some((_, n)) = F_TILDE.iter().find(|(k, _)| *k == key) {
                let m = ev.xterm_mod();
                if m == 1 {
                    out.extend_from_slice(format!("\x1b[{}~", n).as_bytes());
                } else {
                    out.extend_from_slice(format!("\x1b[{};{}~", n, m).as_bytes());
                }
            }
            // Unknown keys encode to nothing.
        }
    }
}

/// Kitty keyboard protocol level-1 (disambiguate) encoding.
fn encode_kitty(ev: &TermKeyEvent, out: &mut Vec<u8>) {
    // Text keys without Ctrl/Alt/Super-class modifiers stay plain text so
    // typing (and IME) is unaffected — that includes Shift.
    if matches!(ev.key(), TermKey::Char) && ev.modifiers & !(modifiers::SHIFT) == 0 {
        encode_legacy(ev, false, out);
        return;
    }

    let codepoint: u16 = match ev.key() {
        TermKey::Char => char::from_u32(ev.codepoint).map(|c| c as u16).unwrap_or(0),
        // TermKey discriminants ARE the kitty functional codes.
        functional => functional as u16,
    };

    // Arrows/Home/End keep their unambiguous legacy CSI <final> form under
    // level 1 when unmodified; everything else goes CSI u.
    if ev.modifiers == 0 {
        if let Some(final_byte) = cursor_group_final(ev.key()) {
            out.push(0x1b);
            out.push(b'[');
            out.push(final_byte);
            return;
        }
    }

    // Kitty omits the modifier field when no modifier is set.
    let m = ev.xterm_mod();
    if m == 1 {
        out.extend_from_slice(format!("\x1b[{}u", codepoint).as_bytes());
    } else {
        out.extend_from_slice(format!("\x1b[{};{}u", codepoint, m).as_bytes());
    }
}

/// Encode a key event against a terminal's negotiated input state.
///
/// Returns the bytes to write to the PTY (or feed back into a mirror).
/// Empty output means the key has no encoding (unknown key).
pub fn encode_key(ev: &TermKeyEvent, term: &Terminal) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    if term.keyboard_flags() & 0x1 != 0 {
        encode_kitty(ev, &mut out);
    } else {
        encode_legacy(ev, term.application_cursor(), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    fn legacy() -> Terminal {
        Terminal::new(80, 24)
    }

    fn enc(ev: &TermKeyEvent, term: &Terminal) -> Vec<u8> {
        encode_key(ev, term)
    }

    #[test]
    fn plain_text_keys() {
        assert_eq!(enc(&TermKeyEvent::char_('a', 0), &legacy()), b"a");
        assert_eq!(
            enc(&TermKeyEvent::char_('A', modifiers::SHIFT), &legacy()),
            b"A"
        );
        assert_eq!(enc(&TermKeyEvent::char_('é', 0), &legacy()), "é".as_bytes());
    }

    #[test]
    fn control_and_alt_letters() {
        assert_eq!(
            enc(&TermKeyEvent::char_('c', modifiers::CTRL), &legacy()),
            b"\x03"
        );
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::CTRL), &legacy()),
            b"\x01"
        );
        assert_eq!(
            enc(&TermKeyEvent::char_(' ', modifiers::CTRL), &legacy()),
            b"\x00"
        );
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::ALT), &legacy()),
            b"\x1ba"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::char_('a', modifiers::CTRL | modifiers::ALT),
                &legacy()
            ),
            b"\x1b\x01"
        );
    }

    #[test]
    fn functional_keys_legacy() {
        let t = legacy();
        assert_eq!(enc(&TermKeyEvent::functional(TermKey::Enter, 0), &t), b"\r");
        assert_eq!(enc(&TermKeyEvent::functional(TermKey::Tab, 0), &t), b"\t");
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Tab, modifiers::SHIFT),
                &t
            ),
            b"\x1b[Z"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Backspace, 0), &t),
            b"\x7f"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Escape, 0), &t),
            b"\x1b"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, 0), &t),
            b"\x1b[A"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, modifiers::CTRL), &t),
            b"\x1b[1;5A"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::F1, 0), &t),
            b"\x1bOP"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::F5, 0), &t),
            b"\x1b[15~"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::F5, modifiers::SHIFT), &t),
            b"\x1b[15;2~"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Delete, modifiers::CTRL),
                &t
            ),
            b"\x1b[3;5~"
        );
    }

    #[test]
    fn application_cursor_uses_ss3() {
        let mut t = legacy();
        // DECCKM — the mode an application sets to opt into SS3 cursor keys.
        t.process(b"\x1b[?1h");
        assert!(t.application_cursor());
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, 0), &t),
            b"\x1bOA"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Home, 0), &t),
            b"\x1bOH"
        );
        // With modifiers the CSI form is used either way.
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, modifiers::CTRL), &t),
            b"\x1b[1;5A"
        );
    }

    #[test]
    fn kitty_disambiguate_mode() {
        let mut t = legacy();
        t.set_keyboard_flags(0x1);
        // Plain typing unaffected.
        assert_eq!(enc(&TermKeyEvent::char_('a', 0), &t), b"a");
        // Ctrl+letter → CSI u.
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::CTRL), &t),
            b"\x1b[97;5u"
        );
        // Enter/Tab/Backspace/Esc always CSI u.
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Enter, 0), &t),
            b"\x1b[13u"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Escape, 0), &t),
            b"\x1b[27u"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Backspace, 0), &t),
            b"\x1b[127u"
        );
        // Arrows unmodified stay CSI <final>.
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, 0), &t),
            b"\x1b[A"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, modifiers::CTRL), &t),
            b"\x1b[57430;5u"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::PageDown, modifiers::SHIFT),
                &t
            ),
            b"\x1b[57433;2u"
        );
    }

    /// QA-151: from_raw covers every discriminant (the macro list cannot
    /// have drifted from the enum) and maps everything else to Unknown.
    #[test]
    fn from_raw_round_trips_every_variant() {
        let all = [
            TermKey::Unknown,
            TermKey::Char,
            TermKey::Escape,
            TermKey::Tab,
            TermKey::Enter,
            TermKey::Backspace,
            TermKey::Insert,
            TermKey::Delete,
            TermKey::Left,
            TermKey::Right,
            TermKey::Up,
            TermKey::Down,
            TermKey::PageUp,
            TermKey::PageDown,
            TermKey::Home,
            TermKey::End,
            TermKey::F1,
            TermKey::F2,
            TermKey::F3,
            TermKey::F4,
            TermKey::F5,
            TermKey::F6,
            TermKey::F7,
            TermKey::F8,
            TermKey::F9,
            TermKey::F10,
            TermKey::F11,
            TermKey::F12,
        ];
        for variant in all {
            assert_eq!(TermKey::from_raw(variant as u16), variant);
        }
        for raw in [2u16, 57388, 0xFFFF] {
            assert_eq!(TermKey::from_raw(raw), TermKey::Unknown, "raw {raw}");
        }
    }
}
