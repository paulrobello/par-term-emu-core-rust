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
//! Super=8, Hyper=16, Meta=32, plus one side-info bit above them.
//!
//! Beyond the two regimes, the legacy encoder covers the modes par-term's
//! own encoder shipped (this crate is now the single encoder — ENH-028):
//!
//! - **modifyOtherKeys** (`CSI > 4 ; Pv m`, Pv 1 or 2): modified text keys
//!   report as `CSI 27 ; mods ; codepoint ~`. Modes 1 and 2 share one rule
//!   set, matching par-term exactly: any text key with Ctrl or Alt held
//!   (Shift-only is exempt, and the base character must be ASCII) switches
//!   to the 27-form even for Ctrl+letter.
//! - **macOS Option (Alt) key modes** — a frontend-owned setting, not
//!   terminal state, passed via [`KeyEncodeOptions`]: Normal (pass the
//!   OS-composed character through), Meta (set the 8th bit on ASCII base
//!   characters), Esc (ESC-prefix the base character). Per side, selected
//!   by the [`modifiers::ALT_RIGHT`] bit.
//!
//! Kitty protocol levels 2-4 (report event types, alternate keys, all keys
//! as escapes, associated text) are not implemented; the kitty branch here
//! is level 1 (disambiguate) only.

use crate::terminal::Terminal;

/// Modifier bitfield — `TermKeyEvent::modifiers`.
pub mod modifiers {
    /// Shift held.
    pub const SHIFT: u8 = 1;
    /// Alt (Option) held.
    pub const ALT: u8 = 2;
    /// Control held.
    pub const CTRL: u8 = 4;
    /// Super (Command) held.
    pub const SUPER: u8 = 8;
    /// Hyper held.
    pub const HYPER: u8 = 16;
    /// Meta held.
    pub const META: u8 = 32;
    /// Side info, not a modifier: the held Alt key is the right one, so
    /// option-key handling uses `right_option`. Absent means the left key —
    /// also what a frontend should send when both are down, since left wins
    /// that tie in par-term. Kitty/legacy modifier parameters ignore this
    /// bit; it lives above the kitty-order bits on purpose.
    pub const ALT_RIGHT: u8 = 64;
}

/// macOS Option (Alt) key handling modes for [`KeyEncodeOptions`] — the
/// same three modes par-term's `OptionKeyMode` config offers.
pub mod option_modes {
    /// Pass the character through untouched (the OS-composed glyph on
    /// macOS, e.g. Option+f → 'ƒ').
    pub const NORMAL: u8 = 0;
    /// Set the 8th bit on ASCII base characters ('a' → 0xE1); non-ASCII
    /// falls back to an ESC prefix.
    pub const META: u8 = 1;
    /// ESC-prefix the base character (ESC a).
    pub const ESC: u8 = 2;
}

/// Frontend-owned key-encoding options (not terminal state). repr(C) so the
/// FFI can pass it as `TermKeyOptions`; a zeroed struct means Normal for
/// both sides.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEncodeOptions {
    /// [`option_modes`] value for the left Option key.
    pub left_option: u8,
    /// [`option_modes`] value for the right Option key.
    pub right_option: u8,
}

impl Default for KeyEncodeOptions {
    /// ESC on both sides — byte-identical to the pre-options encoder
    /// (`encode_key` and `ptec_terminal_encode_key` keep their existing wire
    /// behavior for Alt, the classic xterm ESC prefix).
    fn default() -> Self {
        Self {
            left_option: option_modes::ESC,
            right_option: option_modes::ESC,
        }
    }
}

/// The key pressed. Discriminants double as the kitty protocol functional
/// key codes (`57426` = Insert … `57376..57387` = F1..F12), so the kitty
/// encoder can cast straight through; do not renumber.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKey {
    /// Unrecognized key.
    Unknown = 0,
    /// `codepoint` carries the character
    Char = 1,
    /// Escape.
    Escape = 27,
    /// Tab.
    Tab = 9,
    /// Enter.
    Enter = 13,
    /// Backspace.
    Backspace = 127,
    /// Insert.
    Insert = 57426,
    /// Delete.
    Delete = 57427,
    /// Left arrow.
    Left = 57428,
    /// Right arrow.
    Right = 57429,
    /// Up arrow.
    Up = 57430,
    /// Down arrow.
    Down = 57431,
    /// Page Up.
    PageUp = 57432,
    /// Page Down.
    PageDown = 57433,
    /// Home.
    Home = 57434,
    /// End.
    End = 57435,
    /// Function key F1.
    F1 = 57376,
    /// Function key F2.
    F2 = 57377,
    /// Function key F3.
    F3 = 57378,
    /// Function key F4.
    F4 = 57379,
    /// Function key F5.
    F5 = 57380,
    /// Function key F6.
    F6 = 57381,
    /// Function key F7.
    F7 = 57382,
    /// Function key F8.
    F8 = 57383,
    /// Function key F9.
    F9 = 57384,
    /// Function key F10.
    F10 = 57385,
    /// Function key F11.
    F11 = 57386,
    /// Function key F12.
    F12 = 57387,
}

/// Map a raw wire/header value to a variant; anything that is not a
/// defined discriminant becomes `Unknown` (QA-151). The macro lists the
/// variants once so the match arms stay in lockstep with the enum.
macro_rules! term_key_from_raw {
    ($($variant:ident),* $(,)?) => {
        impl TermKey {
            /// Map a raw `TermKeyEvent.key` value to a variant; any value that is not a defined discriminant becomes [`TermKey::Unknown`].
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

    fn alt_right(&self) -> bool {
        self.modifiers & modifiers::ALT_RIGHT != 0
    }

    fn shift(&self) -> bool {
        self.modifiers & modifiers::SHIFT != 0
    }

    /// Only Shift (or nothing) — the "text-like" modifier set. ALT_RIGHT is
    /// side info, not a modifier, so it must not demote a text key.
    fn text_like(&self) -> bool {
        self.modifiers & !(modifiers::SHIFT | modifiers::ALT_RIGHT) == 0
    }

    /// xterm parameter form: 1 + set modifier bits.
    fn xterm_mod(&self) -> u8 {
        1 + (self.modifiers & 0x3f)
    }

    /// Legacy parameter form counting Shift/Alt/Ctrl only — Super, Hyper,
    /// Meta and the ALT_RIGHT side bit carry no xterm parameter bit.
    fn legacy_mod(&self) -> u8 {
        1 + (self.modifiers & 0x07)
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

/// The option mode for the Alt key the event says is held (right when the
/// ALT_RIGHT side bit is set, left otherwise — the same tie par-term breaks
/// toward left when both keys are down). Unknown mode values pass through
/// as Normal.
fn active_option_mode(ev: &TermKeyEvent, opts: &KeyEncodeOptions) -> u8 {
    let mode = if ev.alt_right() {
        opts.right_option
    } else {
        opts.left_option
    };
    match mode {
        option_modes::META | option_modes::ESC => mode,
        _ => option_modes::NORMAL,
    }
}

/// Append `c` as UTF-8.
fn push_utf8(c: char, out: &mut Vec<u8>) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// Alt-modified text per the option mode (par-term's `apply_option_key_mode`):
/// Normal passes the character through, Meta sets the 8th bit on ASCII bases,
/// Esc prefixes with ESC; non-ASCII bases ESC-prefix in Meta too — there is
/// no high bit to set without corrupting UTF-8.
fn push_alt_text(ev: &TermKeyEvent, c: char, opts: &KeyEncodeOptions, out: &mut Vec<u8>) {
    match active_option_mode(ev, opts) {
        option_modes::META if c.is_ascii() => out.push((c as u8) | 0x80),
        option_modes::NORMAL => push_utf8(c, out),
        // ESC for ASCII bases, and every non-ASCII base in Meta or Esc.
        _ => {
            out.push(0x1b);
            push_utf8(c, out);
        }
    }
}

/// Legacy (xterm) encoding used when no kitty keyboard flags are set.
/// `mok` is the terminal's negotiated modifyOtherKeys mode (0-2).
fn encode_legacy(
    ev: &TermKeyEvent,
    app_cursor: bool,
    mok: u8,
    opts: &KeyEncodeOptions,
    out: &mut Vec<u8>,
) {
    // modifyOtherKeys (modes 1 and 2 share one rule set, matching par-term):
    // text keys with Ctrl or Alt held report as CSI 27;mods;codepoint~ —
    // Shift-only stays exempt so the shifted glyph reaches the app (it
    // cannot be recovered from a base codepoint), and only ASCII bases
    // qualify, mirroring par-term's physical-key table. Functional keys
    // never take this form. Checked first: under modifyOtherKeys the app
    // gets the 27-form even for Ctrl+letter and Alt+letter, so the option
    // modes below do not apply.
    if mok >= 1
        && matches!(ev.key(), TermKey::Char)
        && (ev.ctrl() || ev.alt())
        && char::from_u32(ev.codepoint).is_some_and(|c| c.is_ascii())
    {
        out.extend_from_slice(format!("\x1b[27;{};{}~", ev.legacy_mod(), ev.codepoint).as_bytes());
        return;
    }

    match ev.key() {
        TermKey::Char => {
            let c = char::from_u32(ev.codepoint).unwrap_or('\0');
            if ev.text_like() {
                push_utf8(c, out);
            } else if ev.ctrl() {
                // Ctrl+letter/@.._ → control bytes; Ctrl+Space → NUL.
                let byte = match c.to_ascii_lowercase() {
                    'a'..='z' => (c.to_ascii_lowercase() as u8) - b'a' + 1,
                    '@'..='_' => c as u8 & 0x1f,
                    ' ' => 0,
                    _ => {
                        // Ctrl over a non-control character has no legacy
                        // form; par-term falls back to the plain character,
                        // with the option mode applied when Alt is held.
                        if ev.alt() {
                            push_alt_text(ev, c, opts, out);
                        } else {
                            push_utf8(c, out);
                        }
                        return;
                    }
                };
                // Alt on top of Ctrl: preserve it per the option mode —
                // Meta ORs the high bit onto the control byte, Normal and
                // Esc ESC-prefix it (there is no composed glyph to pass
                // through while Ctrl is held).
                if ev.alt() && active_option_mode(ev, opts) == option_modes::META {
                    out.push(byte | 0x80);
                } else if ev.alt() {
                    out.push(0x1b);
                    out.push(byte);
                } else {
                    out.push(byte);
                }
            } else if ev.alt() {
                // Alt (+Shift) text per the option modes.
                push_alt_text(ev, c, opts, out);
            } else {
                // Super-class modifiers alone do not disturb text keys.
                push_utf8(c, out);
            }
        }
        TermKey::Escape => out.push(0x1b),
        TermKey::Enter => {
            // Shift+Enter is a soft line break (LF) for apps that accept
            // it, like iTerm2; plain Enter is CR.
            if ev.shift() {
                out.push(b'\n');
            } else {
                out.push(b'\r');
            }
        }
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
            let m = ev.legacy_mod();
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
            let m = ev.legacy_mod();
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
                let m = ev.legacy_mod();
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
                let m = ev.legacy_mod();
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
    // Kitty key codes are Unicode scalar values (up to U+10FFFF), so they
    // are u32; a u16 truncated astral codepoints (ARC-098). An unknown key
    // or a Char that is not a scalar value has no encoding, matching the
    // legacy regime's empty output for Unknown.
    let codepoint: u32 = match ev.key() {
        TermKey::Unknown => return,
        TermKey::Char => match char::from_u32(ev.codepoint) {
            Some(c) => c as u32,
            None => return,
        },
        // TermKey discriminants ARE the kitty functional codes.
        functional => functional as u32,
    };

    // Text keys without Ctrl/Alt/Super-class modifiers stay plain text so
    // typing (and IME) is unaffected — that includes Shift, and ALT_RIGHT
    // is side info rather than a modifier.
    if matches!(ev.key(), TermKey::Char) && ev.text_like() {
        encode_legacy(ev, false, 0, &KeyEncodeOptions::default(), out);
        return;
    }

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
    encode_key_with(ev, term, &KeyEncodeOptions::default())
}

/// [`encode_key`] with explicit frontend-owned [`KeyEncodeOptions`] (the
/// macOS Option-key modes). The kitty branch ignores them — the kitty
/// protocol reports the true codepoint and modifiers and lets the
/// application decode Alt itself.
pub fn encode_key_with(ev: &TermKeyEvent, term: &Terminal, opts: &KeyEncodeOptions) -> Vec<u8> {
    encode_with_modes(
        ev,
        term.keyboard_flags(),
        term.application_cursor(),
        term.modify_other_keys_mode(),
        opts,
    )
}

/// [`encode_key`] against a freshly reset terminal's input state (legacy
/// regime, normal cursor keys, no modifyOtherKeys), for callers with no
/// terminal to consult — `macros::KeyParser::parse_key`.
pub(crate) fn encode_key_default(ev: &TermKeyEvent) -> Vec<u8> {
    encode_with_modes(ev, 0, false, 0, &KeyEncodeOptions::default())
}

fn encode_with_modes(
    ev: &TermKeyEvent,
    keyboard_flags: u16,
    app_cursor: bool,
    mok: u8,
    opts: &KeyEncodeOptions,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    if keyboard_flags & 0x1 != 0 {
        encode_kitty(ev, &mut out);
    } else {
        encode_legacy(ev, app_cursor, mok, opts, &mut out);
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

    fn raw_event(key: u16, codepoint: u32, mods: u8) -> TermKeyEvent {
        TermKeyEvent {
            key,
            modifiers: mods,
            _pad: 0,
            codepoint,
        }
    }

    /// ARC-098: kitty key codes are Unicode scalar values; a u16 cast
    /// reported U+1D54F as 54607.
    #[test]
    fn kitty_encodes_astral_codepoint_untruncated() {
        let mut t = legacy();
        t.set_keyboard_flags(0x1);
        assert_eq!(
            enc(&TermKeyEvent::char_('\u{1D54F}', modifiers::CTRL), &t),
            b"\x1b[120143;5u"
        );
    }

    /// ARC-098: an unknown key has no encoding in either regime (kitty
    /// used to emit `CSI 0u`).
    #[test]
    fn kitty_unknown_key_encodes_nothing() {
        let mut kitty = legacy();
        kitty.set_keyboard_flags(0x1);
        let plain = legacy();
        // 0 is Unknown itself; 57437 is not a TermKey discriminant.
        for raw in [0u16, 57437] {
            assert_eq!(TermKey::from_raw(raw), TermKey::Unknown);
            for mods in [0, modifiers::CTRL] {
                let ev = raw_event(raw, 0, mods);
                assert!(enc(&ev, &kitty).is_empty(), "kitty raw {raw} mods {mods}");
                assert!(enc(&ev, &plain).is_empty(), "legacy raw {raw} mods {mods}");
            }
        }
    }

    /// ARC-098: a Char whose codepoint is not a scalar value (a surrogate)
    /// has no kitty encoding (it used to emit `CSI 0;…u`).
    #[test]
    fn kitty_invalid_char_codepoint_encodes_nothing() {
        let mut t = legacy();
        t.set_keyboard_flags(0x1);
        for mods in [0, modifiers::SHIFT, modifiers::CTRL] {
            let ev = raw_event(TermKey::Char as u16, 0xD800, mods);
            assert!(enc(&ev, &t).is_empty(), "mods {mods}");
        }
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

    // ====================================================================
    // ENH-028: modifyOtherKeys + option-key modes. Conformance rows are
    // derived from par-term-input's encoder and its test suite
    // (par-term-input/src/key_encoding.rs and tests/key_encoding_tests.rs,
    // read 2026-09-29) — that encoder shipped first and this crate now has
    // to produce the same bytes so frontends can share one implementation.
    //
    // Model mapping: par-term looks the base character up from the physical
    // key; `TermKeyEvent.codepoint` carries it instead (base form whenever
    // Ctrl/Alt drive the encoding, composed glyph in Normal option mode).
    //
    // Deliberate divergences from par-term, both pre-existing core
    // behavior kept on purpose:
    // - Home/End use SS3 H/F under DECCKM here (xterm/iTerm2 form);
    //   par-term keeps them on CSI.
    // - Alt+Space runs the option transform here; par-term ignores Alt on
    //   Space and always sends " ".
    // ====================================================================

    fn enc_with(ev: &TermKeyEvent, term: &Terminal, opts: &KeyEncodeOptions) -> Vec<u8> {
        encode_key_with(ev, term, opts)
    }

    fn opts_mode(mode: u8) -> KeyEncodeOptions {
        KeyEncodeOptions {
            left_option: mode,
            right_option: mode,
        }
    }

    fn mok(mode: u8) -> Terminal {
        let mut t = Terminal::new(80, 24);
        // The real negotiated path — CSI > 4 ; Pv m.
        t.process(match mode {
            1 => b"\x1b[>4;1m",
            2 => b"\x1b[>4;2m",
            _ => b"\x1b[>4;0m",
        });
        assert_eq!(t.modify_other_keys_mode(), mode);
        t
    }

    /// par-term: `bare_named_keys_use_their_documented_sequences` +
    /// `f5_through_f12_skip_the_keycodes_vt_reserves` — the full bare
    /// functional-key table. Space arrives as a Char event here.
    #[test]
    fn conformance_bare_keys() {
        let t = legacy();
        let cases: &[(TermKeyEvent, &[u8], &str)] = &[
            (TermKeyEvent::functional(TermKey::Up, 0), b"\x1b[A", "Up"),
            (
                TermKeyEvent::functional(TermKey::Down, 0),
                b"\x1b[B",
                "Down",
            ),
            (
                TermKeyEvent::functional(TermKey::Right, 0),
                b"\x1b[C",
                "Right",
            ),
            (
                TermKeyEvent::functional(TermKey::Left, 0),
                b"\x1b[D",
                "Left",
            ),
            (
                TermKeyEvent::functional(TermKey::Home, 0),
                b"\x1b[H",
                "Home",
            ),
            (TermKeyEvent::functional(TermKey::End, 0), b"\x1b[F", "End"),
            (
                TermKeyEvent::functional(TermKey::Insert, 0),
                b"\x1b[2~",
                "Insert",
            ),
            (
                TermKeyEvent::functional(TermKey::Delete, 0),
                b"\x1b[3~",
                "Delete",
            ),
            (
                TermKeyEvent::functional(TermKey::PageUp, 0),
                b"\x1b[5~",
                "PageUp",
            ),
            (
                TermKeyEvent::functional(TermKey::PageDown, 0),
                b"\x1b[6~",
                "PageDown",
            ),
            (TermKeyEvent::functional(TermKey::F1, 0), b"\x1bOP", "F1"),
            (TermKeyEvent::functional(TermKey::F2, 0), b"\x1bOQ", "F2"),
            (TermKeyEvent::functional(TermKey::F3, 0), b"\x1bOR", "F3"),
            (TermKeyEvent::functional(TermKey::F4, 0), b"\x1bOS", "F4"),
            (TermKeyEvent::functional(TermKey::F5, 0), b"\x1b[15~", "F5"),
            (TermKeyEvent::functional(TermKey::F6, 0), b"\x1b[17~", "F6"),
            (TermKeyEvent::functional(TermKey::F7, 0), b"\x1b[18~", "F7"),
            (TermKeyEvent::functional(TermKey::F8, 0), b"\x1b[19~", "F8"),
            (TermKeyEvent::functional(TermKey::F9, 0), b"\x1b[20~", "F9"),
            (
                TermKeyEvent::functional(TermKey::F10, 0),
                b"\x1b[21~",
                "F10",
            ),
            (
                TermKeyEvent::functional(TermKey::F11, 0),
                b"\x1b[23~",
                "F11",
            ),
            (
                TermKeyEvent::functional(TermKey::F12, 0),
                b"\x1b[24~",
                "F12",
            ),
            (TermKeyEvent::functional(TermKey::Enter, 0), b"\r", "Enter"),
            (TermKeyEvent::functional(TermKey::Tab, 0), b"\t", "Tab"),
            (TermKeyEvent::char_(' ', 0), b" ", "Space"),
            (
                TermKeyEvent::functional(TermKey::Backspace, 0),
                b"\x7f",
                "Backspace",
            ),
            (
                TermKeyEvent::functional(TermKey::Escape, 0),
                b"\x1b",
                "Escape",
            ),
        ];
        for (ev, expected, what) in cases {
            assert_eq!(&enc(ev, &t), expected, "{what}");
        }
        // Tilde keycodes deliberately skip 16 and 22 — xterm never assigned
        // them (par-term pins this too).
        for key in [TermKey::F5, TermKey::F6, TermKey::F11] {
            let bytes = enc(&TermKeyEvent::functional(key, 0), &t);
            let text = String::from_utf8(bytes).unwrap();
            assert_ne!(text, "\x1b[16~", "{key:?} must not use reserved 16");
            assert_ne!(text, "\x1b[22~", "{key:?} must not use reserved 22");
        }
    }

    /// par-term: `arrow_keys_encode_every_modifier_combination` +
    /// `tilde_form_and_f1_to_f4_take_the_same_modifier_parameter` +
    /// `super_alone_is_not_an_xterm_modifier`. The legacy modifier
    /// parameter counts Shift/Alt/Ctrl only — Super/Hyper/Meta (and the
    /// ALT_RIGHT side bit) must not reach it.
    #[test]
    fn conformance_functional_key_modifiers() {
        let t = legacy();
        let cases: &[(u8, u8)] = &[
            (modifiers::SHIFT, 2),
            (modifiers::ALT, 3),
            (modifiers::SHIFT | modifiers::ALT, 4),
            (modifiers::CTRL, 5),
            (modifiers::SHIFT | modifiers::CTRL, 6),
            (modifiers::ALT | modifiers::CTRL, 7),
            (modifiers::SHIFT | modifiers::ALT | modifiers::CTRL, 8),
        ];
        for (mods, param) in cases {
            for (key, suffix) in [
                (TermKey::Up, 'A'),
                (TermKey::Down, 'B'),
                (TermKey::Right, 'C'),
                (TermKey::Left, 'D'),
                (TermKey::Home, 'H'),
                (TermKey::End, 'F'),
            ] {
                let expected = format!("\x1b[1;{param}{suffix}");
                assert_eq!(
                    enc(&TermKeyEvent::functional(key, *mods), &t),
                    expected.as_bytes(),
                    "{key:?} with mods {mods:?}"
                );
            }
        }
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Delete, modifiers::SHIFT),
                &t
            ),
            b"\x1b[3;2~",
            "Shift+Delete"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::PageUp, modifiers::CTRL),
                &t
            ),
            b"\x1b[5;5~",
            "Ctrl+PageUp"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::F12, modifiers::SHIFT | modifiers::CTRL),
                &t
            ),
            b"\x1b[24;6~",
            "Ctrl+Shift+F12"
        );
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::F1, modifiers::SHIFT), &t),
            b"\x1b[1;2P",
            "Shift+F1"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::F4, modifiers::ALT | modifiers::CTRL),
                &t
            ),
            b"\x1b[1;7S",
            "Ctrl+Alt+F4"
        );
        // Super carries no xterm parameter bit: Cmd+Up encodes as bare Up
        // (Cmd shortcuts are intercepted above this layer), and a held
        // right-Alt side bit must not fabricate a modifier either.
        assert_eq!(
            enc(&TermKeyEvent::functional(TermKey::Up, modifiers::SUPER), &t),
            b"\x1b[A",
            "Super+Up"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Up, modifiers::SUPER | modifiers::CTRL),
                &t
            ),
            b"\x1b[1;5A",
            "Ctrl+Super+Up counts Ctrl only"
        );
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Up, modifiers::ALT_RIGHT),
                &t
            ),
            b"\x1b[A",
            "bare ALT_RIGHT side bit is not a modifier"
        );
    }

    /// par-term: `ctrl_letter_maps_to_control_codes` +
    /// `ctrl_punctuation_in_the_0x40_to_0x5f_range_maps_to_control_codes` +
    /// `ctrl_space_sends_nul` + `ctrl_question_mark_does_not_send_del`.
    /// The codepoint is the base form; Shift on top of Ctrl must not change
    /// the control byte.
    #[test]
    fn conformance_ctrl_characters() {
        let t = legacy();
        let letters = [
            ('a', 0x01u8),
            ('b', 0x02),
            ('c', 0x03),
            ('i', 0x09),
            ('m', 0x0d),
            ('z', 0x1a),
        ];
        for (c, expected) in letters {
            assert_eq!(
                enc(&TermKeyEvent::char_(c, modifiers::CTRL), &t),
                &[expected],
                "Ctrl+{c}"
            );
            assert_eq!(
                enc(
                    &TermKeyEvent::char_(c, modifiers::CTRL | modifiers::SHIFT),
                    &t
                ),
                &[expected],
                "Ctrl+Shift+{c}"
            );
        }
        let punctuation = [
            ('@', 0x00u8),
            ('[', 0x1b),
            ('\\', 0x1c),
            (']', 0x1d),
            ('^', 0x1e),
            ('_', 0x1f),
        ];
        for (c, expected) in punctuation {
            assert_eq!(
                enc(&TermKeyEvent::char_(c, modifiers::CTRL), &t),
                &[expected],
                "Ctrl+{c}"
            );
        }
        assert_eq!(
            enc(&TermKeyEvent::char_(' ', modifiers::CTRL), &t),
            &[0x00],
            "Ctrl+Space"
        );
        // Divergence pinned by par-term too: Ctrl+? sends the literal '?'
        // (xterm would send DEL).
        assert_eq!(
            enc(&TermKeyEvent::char_('?', modifiers::CTRL), &t),
            b"?",
            "Ctrl+?"
        );
    }

    /// par-term: `ctrl_non_ascii_char_is_sent_as_utf8_not_truncated` — a
    /// non-ASCII codepoint with Ctrl held must reach the PTY as its UTF-8
    /// bytes, never masked to a control code by its low byte.
    #[test]
    fn conformance_ctrl_non_ascii() {
        let t = legacy();
        for c in ['ł', 'ŀ', 'ŕ', '🍀', 'é', '日'] {
            assert_eq!(
                enc(&TermKeyEvent::char_(c, modifiers::CTRL), &t),
                c.to_string().as_bytes(),
                "Ctrl+{c}"
            );
        }
    }

    /// par-term: the ENH-002 unicode corpus rows — unmodified character
    /// keys write their UTF-8 bytes verbatim, and Shift/Super never
    /// disturb them at any modifyOtherKeys level.
    #[test]
    fn conformance_unicode_verbatim() {
        const CORPUS: &[&str] = &[
            "é",
            "café",
            "Привет",
            "ΟΔΟΣ",
            "日本語",
            "한글",
            "😀",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{1F1EF}\u{1F1F5}",
            "\u{1F44D}\u{1F3FD}",
            "e\u{0301}",
            "مرحبا",
            "שלום",
            "\u{200D}",
            "\u{201C}x\u{201D}",
            "a日b😀c",
        ];
        for mode in [0u8, 1, 2] {
            let t = mok(mode);
            for text in CORPUS {
                for mods in [
                    0u8,
                    modifiers::SHIFT,
                    modifiers::SUPER,
                    modifiers::SHIFT | modifiers::SUPER,
                ] {
                    let c = text.chars().next().expect("non-empty");
                    assert_eq!(
                        enc(&TermKeyEvent::char_(c, mods), &t),
                        c.to_string().as_bytes(),
                        "{text:?} first char with mods {mods} at mode {mode}"
                    );
                }
            }
        }
    }

    /// par-term: `option_key_modes_transform_an_ascii_base_character` +
    /// `option_key_modes_fall_back_to_esc_prefixing_for_non_ascii`. The
    /// composed glyph ('ƒ') passes through in Normal; Meta/Esc transform
    /// the base form ('f'); non-ASCII bases ESC-prefix rather than corrupt
    /// a multi-byte character.
    #[test]
    fn conformance_option_key_modes() {
        let t = legacy();
        // Normal: OS-composed character verbatim (Option+f = ƒ on macOS).
        assert_eq!(
            enc_with(
                &TermKeyEvent::char_('ƒ', modifiers::ALT),
                &t,
                &opts_mode(option_modes::NORMAL)
            ),
            "ƒ".as_bytes(),
            "Normal passes the composed glyph through"
        );
        // Meta: 8th bit on the base form ('f' | 0x80 = 0xE6).
        assert_eq!(
            enc_with(
                &TermKeyEvent::char_('f', modifiers::ALT),
                &t,
                &opts_mode(option_modes::META)
            ),
            &[0xE6],
            "Meta sets the high bit on the base character"
        );
        // Esc: ESC then the base character.
        assert_eq!(
            enc_with(
                &TermKeyEvent::char_('f', modifiers::ALT),
                &t,
                &opts_mode(option_modes::ESC)
            ),
            &[0x1b, b'f'],
            "Esc prefixes the base character"
        );
        // Non-ASCII base: Meta and Esc both ESC-prefix (Meta cannot set a
        // high bit without corrupting UTF-8).
        for mode in [option_modes::META, option_modes::ESC] {
            assert_eq!(
                enc_with(
                    &TermKeyEvent::char_('é', modifiers::ALT),
                    &t,
                    &opts_mode(mode)
                ),
                [&[0x1b][..], "é".as_bytes()].concat(),
                "{mode:?} ESC-prefixes a non-ASCII base"
            );
        }
        assert_eq!(
            enc_with(
                &TermKeyEvent::char_('é', modifiers::ALT),
                &t,
                &opts_mode(option_modes::NORMAL)
            ),
            "é".as_bytes(),
            "Normal leaves a non-ASCII character alone"
        );
        // Default options are ESC on both sides — the classic xterm Alt.
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::ALT), &t),
            b"\x1ba",
            "default options keep the legacy ESC prefix"
        );
    }

    /// Side selection: the ALT_RIGHT bit routes to `right_option`. Absent
    /// (or both keys held, which a frontend reports as left) → left.
    #[test]
    fn option_mode_side_selection() {
        let t = legacy();
        let opts = KeyEncodeOptions {
            left_option: option_modes::NORMAL,
            right_option: option_modes::META,
        };
        let left = TermKeyEvent::char_('a', modifiers::ALT);
        let right = TermKeyEvent::char_('a', modifiers::ALT | modifiers::ALT_RIGHT);
        assert_eq!(enc_with(&left, &t, &opts), b"a", "left key → left mode");
        assert_eq!(
            enc_with(&right, &t, &opts),
            &[0xE1],
            "right key → right mode"
        );
        // Default options: ALT_RIGHT alone never fabricates output.
        assert_eq!(
            enc_with(&right, &t, &KeyEncodeOptions::default()),
            b"\x1ba",
            "right key under ESC defaults still ESC-prefixes"
        );
    }

    /// par-term: `ctrl_alt_letter_preserves_the_alt_modifier` — Ctrl+Alt of
    /// a control-capable key ORs the high bit (Meta) or ESC-prefixes the
    /// control byte (Normal AND Esc — Normal has no composed form worth
    /// passing through when Ctrl is held).
    #[test]
    fn conformance_ctrl_alt() {
        let t = legacy();
        let ev = TermKeyEvent::char_('a', modifiers::CTRL | modifiers::ALT);
        assert_eq!(
            enc_with(&ev, &t, &opts_mode(option_modes::META)),
            &[0x81],
            "Meta ORs the high bit onto the control byte"
        );
        for mode in [option_modes::NORMAL, option_modes::ESC] {
            assert_eq!(
                enc_with(&ev, &t, &opts_mode(mode)),
                &[0x1b, 0x01],
                "{mode:?} prefixes the control byte with ESC"
            );
        }
        // Without options the legacy ESC-prefix behavior is unchanged.
        assert_eq!(enc(&ev, &t), &[0x1b, 0x01]);
    }

    /// par-term: `modify_other_keys_reports_the_base_codepoint_not_the_shifted_one`
    /// plus the mode-1/mode-2 rule set: both modes share one path in
    /// par-term (Ctrl or Alt held, ASCII base, Shift-only exempt).
    #[test]
    fn conformance_modify_other_keys() {
        // Ctrl+Shift+1 → CSI 27;6;49~ (base '1', not shifted '!').
        let t2 = mok(2);
        assert_eq!(
            enc(
                &TermKeyEvent::char_('1', modifiers::CTRL | modifiers::SHIFT),
                &t2
            ),
            b"\x1b[27;6;49~",
            "mode 2 reports the base codepoint"
        );
        // Mode 1 behaves identically (par-term parity).
        let t1 = mok(1);
        assert_eq!(
            enc(&TermKeyEvent::char_('c', modifiers::CTRL), &t1),
            b"\x1b[27;5;99~",
            "mode 1 reports Ctrl+letter too"
        );
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::CTRL), &t2),
            b"\x1b[27;5;97~"
        );
        // Ctrl+Space reports the space codepoint.
        assert_eq!(
            enc(&TermKeyEvent::char_(' ', modifiers::CTRL), &t1),
            b"\x1b[27;5;32~"
        );
        // Alt also routes here — the option mode does not apply under
        // modifyOtherKeys.
        assert_eq!(
            enc(&TermKeyEvent::char_('f', modifiers::ALT), &t2),
            b"\x1b[27;3;102~"
        );
        // Shift-only is exempt: the shifted glyph goes out verbatim.
        assert_eq!(enc(&TermKeyEvent::char_('1', modifiers::SHIFT), &t2), b"1");
        // Non-ASCII base: no 27-form, fall through to plain text.
        assert_eq!(
            enc(&TermKeyEvent::char_('é', modifiers::CTRL), &t2),
            "é".as_bytes()
        );
        // Functional keys never take the 27-form (par-term's base-char
        // table has no entries for them).
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Enter, modifiers::CTRL),
                &t2
            ),
            b"\r"
        );
    }

    /// par-term: `modify_other_keys_level_zero_never_emits_csi_27`.
    #[test]
    fn conformance_modify_other_keys_level_zero() {
        let t = legacy();
        let texts = ["a", "é", "caf\u{e9}", "日", "\u{1F44D}", "?", "1"];
        for text in texts {
            let c = text.chars().next().unwrap();
            for mods in [
                modifiers::CTRL,
                modifiers::ALT,
                modifiers::CTRL | modifiers::ALT,
                modifiers::SHIFT | modifiers::CTRL,
            ] {
                let bytes = enc(&TermKeyEvent::char_(c, mods), &t);
                assert!(
                    !bytes.starts_with(b"\x1b[27;"),
                    "level 0 must not use modifyOtherKeys for {text:?} with {mods:?}"
                );
            }
        }
    }

    /// Shift+Enter is a soft line break (LF) like iTerm2; plain Enter is CR.
    #[test]
    fn conformance_shift_enter() {
        let t = legacy();
        assert_eq!(
            enc(
                &TermKeyEvent::functional(TermKey::Enter, modifiers::SHIFT),
                &t
            ),
            b"\n",
            "Shift+Enter sends LF"
        );
        assert_eq!(enc(&TermKeyEvent::functional(TermKey::Enter, 0), &t), b"\r");
    }

    /// ENH-028 criterion: kitty level 1 stays byte-identical, and the new
    /// option modes must not leak into the kitty branch (the kitty protocol
    /// reports the true codepoint + modifiers and lets the application
    /// decode Alt itself).
    #[test]
    fn conformance_kitty_unchanged() {
        let mut t = Terminal::new(80, 24);
        t.set_keyboard_flags(0x1);
        let any = KeyEncodeOptions {
            left_option: option_modes::META,
            right_option: option_modes::META,
        };
        assert_eq!(
            enc_with(&TermKeyEvent::char_('a', 0), &t, &any),
            b"a",
            "plain typing unaffected"
        );
        assert_eq!(
            enc_with(&TermKeyEvent::char_('a', modifiers::CTRL), &t, &any),
            b"\x1b[97;5u"
        );
        assert_eq!(
            enc_with(&TermKeyEvent::char_('a', modifiers::ALT), &t, &any),
            b"\x1b[97;3u",
            "kitty reports Alt as a modifier, not via option modes"
        );
        assert_eq!(
            enc_with(
                &TermKeyEvent::char_('a', modifiers::ALT | modifiers::ALT_RIGHT),
                &t,
                &any
            ),
            b"\x1b[97;3u",
            "ALT_RIGHT is side info only — it must not perturb the kitty mods field"
        );
        assert_eq!(
            enc_with(&TermKeyEvent::functional(TermKey::Enter, 0), &t, &any),
            b"\x1b[13u"
        );
        assert_eq!(
            enc_with(
                &TermKeyEvent::functional(TermKey::Up, modifiers::CTRL),
                &t,
                &any
            ),
            b"\x1b[57430;5u"
        );
    }

    /// The wire sequence `CSI > 4 ; Pv m` resets too (XTRESETMODIFYOTHERKEYS
    /// is `CSI > 4 m`), so the encoder reads the negotiated state live.
    #[test]
    fn modify_other_keys_reset() {
        let mut t = Terminal::new(80, 24);
        t.process(b"\x1b[>4;2m");
        assert_eq!(t.modify_other_keys_mode(), 2);
        assert_eq!(
            enc(&TermKeyEvent::char_('a', modifiers::CTRL), &t),
            b"\x1b[27;5;97~"
        );
        t.process(b"\x1b[>4m");
        assert_eq!(t.modify_other_keys_mode(), 0);
        assert_eq!(enc(&TermKeyEvent::char_('a', modifiers::CTRL), &t), b"\x01");
    }

    /// Encoding never panics and never emits a truncated multi-byte
    /// character across the option modes (par-term's fuzz invariant, as a
    /// deterministic sweep — every char × mode × modifier combination).
    #[test]
    fn encoding_sweep_never_truncates_utf8() {
        let chars = ['a', 'A', '1', '?', 'é', 'ł', '日', '😀', '\u{0301}'];
        let modes = [
            0u8,
            option_modes::NORMAL,
            option_modes::META,
            option_modes::ESC,
            0xFF,
        ];
        let mod_sets = [
            0u8,
            modifiers::SHIFT,
            modifiers::CTRL,
            modifiers::ALT,
            modifiers::ALT_RIGHT,
            modifiers::SHIFT | modifiers::CTRL,
            modifiers::CTRL | modifiers::ALT,
            modifiers::ALT | modifiers::ALT_RIGHT,
            modifiers::CTRL | modifiers::ALT | modifiers::ALT_RIGHT,
            modifiers::SUPER,
        ];
        for &mok_mode in &[0u8, 1, 2] {
            let t = mok(mok_mode);
            for &c in &chars {
                for &mode in &modes {
                    for &mods in &mod_sets {
                        let ev = TermKeyEvent::char_(c, mods);
                        let bytes = enc_with(
                            &ev,
                            &t,
                            &KeyEncodeOptions {
                                left_option: mode,
                                right_option: mode,
                            },
                        );
                        let payload = bytes.strip_prefix(&[0x1b]).unwrap_or(&bytes);
                        if payload.iter().any(|b| *b >= 0x80) {
                            assert!(
                                std::str::from_utf8(payload).is_ok() || payload.len() == 1,
                                "truncated {c:?} with mods {mods} mode {mode} mok {mok_mode}: {bytes:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
