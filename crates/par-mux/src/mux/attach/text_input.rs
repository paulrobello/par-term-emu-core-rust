//! Non-ASCII (IME / CJK) input and display-width helpers for the attach
//! client's own consumers: the prefix scanner, the modal text fields,
//! and the chrome rows that lay names out by cell.
//!
//! The host terminal owns IME composition (preedit) for every TUI: the
//! client only ever reads the COMMITTED text, as UTF-8 bytes on stdin.
//! Those bytes arrive one at a time through the byte routers, so a
//! multi-byte character needs reassembly before it can be judged.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// One [`Utf8Assembler::push`] outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Utf8Step {
    /// A lead or continuation byte of an unfinished character.
    Pending,
    /// The character the byte completed.
    Char(char),
    /// Not part of a well-formed character: an ASCII byte, a stray
    /// continuation, an invalid lead, or a sequence `str::from_utf8`
    /// rejects (overlong, surrogate, past U+10FFFF). The partial is
    /// discarded.
    Invalid,
}

/// Byte-at-a-time UTF-8 reassembly. A new lead byte discards any
/// unfinished partial (the input resynchronizes at the next character).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Utf8Assembler {
    buf: [u8; 4],
    len: u8,
    want: u8,
}

impl Utf8Assembler {
    /// Feed one byte.
    pub(crate) fn push(&mut self, byte: u8) -> Utf8Step {
        let width = match byte {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            0x80..=0xBF if self.want > 0 => {
                self.buf[usize::from(self.len)] = byte;
                self.len += 1;
                if self.len < self.want {
                    return Utf8Step::Pending;
                }
                let decoded = std::str::from_utf8(&self.buf[..usize::from(self.len)])
                    .ok()
                    .and_then(|s| s.chars().next());
                *self = Self::default();
                return decoded.map_or(Utf8Step::Invalid, Utf8Step::Char);
            }
            _ => {
                *self = Self::default();
                return Utf8Step::Invalid;
            }
        };
        *self = Self {
            buf: [byte, 0, 0, 0],
            len: 1,
            want: width,
        };
        Utf8Step::Pending
    }

    /// Feed one byte `>= 0x80` from a text field: the completed
    /// character when it is printable (C1 controls and other control
    /// characters never enter a field).
    pub(crate) fn push_printable(&mut self, byte: u8) -> Option<char> {
        match self.push(byte) {
            Utf8Step::Char(ch) if !ch.is_control() => Some(ch),
            _ => None,
        }
    }
}

/// The ASCII byte a fullwidth form (U+FF01..=U+FF5E) stands for. A CJK
/// IME in fullwidth mode commits `ｃ` / `％` without a preedit step, so
/// the prefix scanner folds them back to the chord bytes the user meant.
/// The mapping is the Unicode Halfwidth and Fullwidth Forms block's fixed
/// 0xFEE0 offset; nothing outside it folds.
pub(crate) fn fold_fullwidth_ascii(ch: char) -> Option<u8> {
    let cp = u32::from(ch);
    (0xFF01..=0xFF5E)
        .contains(&cp)
        .then(|| u8::try_from(cp - 0xFEE0).expect("block maps into ASCII"))
}

/// The cells `s` occupies on a terminal (wide CJK = 2).
pub(crate) fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// The cells one character occupies (control and zero-width = 0).
pub(crate) fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// The longest prefix of `s` that fits in `budget` cells, and its width.
/// A wide character that would straddle the budget is left out whole.
pub(crate) fn truncate_to_width(s: &str, budget: usize) -> (&str, usize) {
    let mut used = 0;
    for (index, ch) in s.char_indices() {
        let w = char_width(ch);
        if used + w > budget {
            return (&s[..index], used);
        }
        used += w;
    }
    (s, used)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(bytes: &[u8]) -> Vec<Utf8Step> {
        let mut a = Utf8Assembler::default();
        bytes.iter().map(|&b| a.push(b)).collect()
    }

    #[test]
    fn assembles_two_three_and_four_byte_characters() {
        use Utf8Step::{Char, Pending};
        assert_eq!(feed("é".as_bytes()), [Pending, Char('é')]);
        assert_eq!(feed("中".as_bytes()), [Pending, Pending, Char('中')]);
        assert_eq!(
            feed("😀".as_bytes()),
            [Pending, Pending, Pending, Char('😀')]
        );
    }

    #[test]
    fn malformed_input_is_invalid_and_resynchronizes() {
        use Utf8Step::{Char, Invalid, Pending};
        // A stray continuation, an invalid lead, ASCII.
        assert_eq!(feed(&[0x80]), [Invalid]);
        assert_eq!(feed(&[0xC0, 0xFF]), [Invalid, Invalid]);
        assert_eq!(feed(b"a"), [Invalid]);
        // A surrogate (ED A0 80) passes the shape check but not from_utf8.
        assert_eq!(feed(&[0xED, 0xA0, 0x80]), [Pending, Pending, Invalid]);
        // A new lead mid-character drops the partial and starts over.
        let mut bytes = vec![0xE4];
        bytes.extend_from_slice("文".as_bytes());
        assert_eq!(feed(&bytes), [Pending, Pending, Pending, Char('文')]);
    }

    #[test]
    fn push_printable_rejects_c1_controls() {
        let mut a = Utf8Assembler::default();
        // U+0085 NEL is C2 85.
        assert_eq!(a.push_printable(0xC2), None);
        assert_eq!(a.push_printable(0x85), None);
        for &b in "名".as_bytes() {
            if let Some(ch) = a.push_printable(b) {
                assert_eq!(ch, '名');
            }
        }
    }

    #[test]
    fn fullwidth_forms_fold_to_ascii_and_nothing_else_does() {
        assert_eq!(fold_fullwidth_ascii('ｃ'), Some(b'c'));
        assert_eq!(fold_fullwidth_ascii('％'), Some(b'%'));
        assert_eq!(fold_fullwidth_ascii('＂'), Some(b'"'));
        assert_eq!(fold_fullwidth_ascii('！'), Some(b'!'));
        assert_eq!(fold_fullwidth_ascii('～'), Some(b'~'));
        assert_eq!(fold_fullwidth_ascii('中'), None);
        assert_eq!(fold_fullwidth_ascii('\u{3000}'), None);
        assert_eq!(fold_fullwidth_ascii('｟'), None);
    }

    #[test]
    fn truncation_counts_cells_and_never_splits_a_wide_char() {
        assert_eq!(display_width("日本語"), 6);
        assert_eq!(truncate_to_width("日本語", 5), ("日本", 4));
        assert_eq!(truncate_to_width("日本語", 6), ("日本語", 6));
        assert_eq!(truncate_to_width("ab日", 3), ("ab", 2));
        assert_eq!(truncate_to_width("abc", 0), ("", 0));
    }
}
