//! The Phase B render-mode input router: parse the host's raw stdin byte
//! stream, then re-encode and route every token through the focused pane's
//! tracked input state.
//!
//! Why a hand-rolled parser: crossterm's event parser is `pub(crate)`, and
//! its blocking reader thread would race the client's own [`super::Stdin`]
//! pump (two readers on one tty). This parser consumes the same byte
//! stream the [`super::Stdin`] thread already delivers, incrementally — a
//! sequence split across two reads is held pending until complete.
//!
//! Tokens:
//!
//! - plain byte runs (typed text, control bytes) — forwarded verbatim,
//!   after the prefix scan, because the host already encoded them;
//! - escape-sequence keys — decoded to a [`TermKeyEvent`] and RE-ENCODED
//!   against the focused pane's terminal (`crate::keyboard::encode_key`
//!   honors the pane's DECCKM application-cursor mode and kitty keyboard
//!   flags tracked from the replay + `%output` stream), so a pane running
//!   vim gets `ESC O A` while a plain shell gets `ESC [ A`;
//! - SGR mouse reports — routed by the render session ([`super::render`]):
//!   clicks focus the pane under the pointer, and events are forwarded
//!   pane-relative when that pane owns mouse tracking.
//!
//! Kitty keyboard protocol forwarding is out of scope for this card: the
//! re-encoder emits the legacy/kitty-level-1 forms `encode_key` produces
//! and never negotiates flags on the pane's behalf. Full forwarding is a
//! later Phase B card (documented in docs/MUX.md).

use crate::keyboard::{modifiers, TermKey, TermKeyEvent};
use crate::terminal::Terminal;

/// One parsed unit of host stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A run of plain bytes (no escape sequences): text and control bytes
    /// the host terminal already encoded. Forwarded verbatim behind the
    /// prefix scan.
    Bytes(Vec<u8>),
    /// A decoded escape-sequence key press — re-encoded against the
    /// focused pane's input modes before forwarding.
    Key(TermKeyEvent),
    /// An SGR mouse report from the host (`CSI < cb ; col ; row M/m`),
    /// coordinates 1-based exactly as on the wire, `cb` the raw SGR button
    /// code (motion bit 32, wheel 64/65, modifiers above bit 2).
    Mouse(SgrMouse),
}

/// An SGR mouse report as the host terminal sent it (1-based coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SgrMouse {
    /// The raw SGR button code.
    pub cb: u8,
    /// 1-based column.
    pub col: u16,
    /// 1-based row.
    pub row: u16,
    /// `true` for a release report (`m`), `false` for press/motion (`M`).
    pub release: bool,
}

impl SgrMouse {
    /// Wheel-up report (button code 64)?
    pub fn is_wheel_up(&self) -> bool {
        self.cb & !0x3 & 0x40 != 0 && self.cb & 0x3 == 0
    }

    /// Wheel-down report (button code 65)?
    pub fn is_wheel_down(&self) -> bool {
        self.cb & !0x3 & 0x40 != 0 && self.cb & 0x3 == 1
    }

    /// A motion/drag report (motion bit set, not a wheel report)?
    pub fn is_motion(&self) -> bool {
        self.cb & 32 != 0 && !self.is_wheel_up() && !self.is_wheel_down()
    }

    /// The pressed-button index (0 left, 1 middle, 2 right); the low two
    /// bits of the button code.
    pub fn button(&self) -> u8 {
        self.cb & 0x3
    }

    /// The modifier bits, in the crate's [`modifiers`] order — the SGR
    /// wire puts them at shift 2 with the same bit values (shift 1, alt 2,
    /// ctrl 4).
    pub fn modifiers(&self) -> u8 {
        (self.cb >> 2) & 0x7
    }

    /// Re-encode as a pane-relative SGR sequence: coordinates rebased to
    /// the pane's own 0-based origin. Always SGR, per tmux's
    /// re-encode-to-the-pane behavior with the modern encoding every mouse
    /// app negotiates (legacy pane encodings are a documented
    /// simplification).
    pub fn reencode_sgr(&self, rel_col: u16, rel_row: u16) -> Vec<u8> {
        let event = crate::mouse::MouseEvent::new(
            self.cb,
            rel_col as usize,
            rel_row as usize,
            !self.release,
            0,
        );
        event.encode(
            crate::mouse::MouseMode::AnyEvent,
            crate::mouse::MouseEncoding::Sgr,
        )
    }
}

/// Incremental stdin tokenizer: feed raw bytes, take complete tokens. A
/// partial escape sequence at the end of a burst stays pending until the
/// next feed completes it; a lone ESC (no following byte this burst) is
/// emitted as the Escape key at once — the classic interactivity trade,
/// and what xterm does with no timeout configured.
#[derive(Default)]
pub struct InputParser {
    pending: Vec<u8>,
}

impl InputParser {
    /// Feed one stdin burst, returning every complete token in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Token> {
        self.pending.extend_from_slice(bytes);
        let mut tokens = Vec::new();
        let mut plain = Vec::new();
        let data = std::mem::take(&mut self.pending);
        let mut i = 0;
        while i < data.len() {
            if data[i] != 0x1b {
                plain.push(data[i]);
                i += 1;
                continue;
            }
            // An escape sequence: flush the plain run first so token order
            // matches the wire order.
            if !plain.is_empty() {
                tokens.push(Token::Bytes(std::mem::take(&mut plain)));
            }
            match self.scan_escape(&data[i..]) {
                EscapeScan::Token(token, consumed) => {
                    tokens.push(token);
                    i += consumed;
                }
                EscapeScan::Incomplete(consumed) => {
                    // Hold the partial sequence for the next feed. A lone
                    // trailing ESC (nothing after it at all) is delivered
                    // as the Escape key now, and the ESC byte is consumed
                    // (the loop advances past it).
                    if consumed == 1 {
                        tokens.push(Token::Key(TermKeyEvent::functional(TermKey::Escape, 0)));
                        i += 1;
                    } else {
                        self.pending = data[i..].to_vec();
                        return tokens;
                    }
                }
            }
        }
        if !plain.is_empty() {
            tokens.push(Token::Bytes(plain));
        }
        tokens
    }

    /// Scan one escape sequence at `data[0] == ESC`. `Incomplete(n)` means
    /// the sequence needs more bytes; `n` is 1 for a lone trailing ESC,
    /// else the byte count already visible.
    fn scan_escape(&mut self, data: &[u8]) -> EscapeScan {
        if data.len() < 2 {
            return EscapeScan::Incomplete(1);
        }
        match data[1] {
            b'[' => self.scan_csi(data),
            b'O' => {
                if data.len() < 3 {
                    return EscapeScan::Incomplete(data.len());
                }
                match ss3_key(data[2], 0) {
                    Some(key) => EscapeScan::Token(Token::Key(TermKeyEvent::functional(key, 0)), 3),
                    None => EscapeScan::Token(Token::Bytes(vec![0x1b, b'O', data[2]]), 3),
                }
            }
            // ESC ESC: a standalone Escape key press followed by another
            // sequence (the Alt-self-insert spelling some terminals send).
            0x1b => EscapeScan::Token(Token::Key(TermKeyEvent::functional(TermKey::Escape, 0)), 1),
            // ESC + a printable/other byte: Alt+byte. Re-encode through the
            // key encoder so the pane's option/meta mode applies — an Alt
            // chord is a Key event on the char.
            other => EscapeScan::Token(
                Token::Key(TermKeyEvent::char_(other as char, modifiers::ALT)),
                2,
            ),
        }
    }

    /// Scan `ESC [ …`: either an SGR mouse report or a decoded key
    /// sequence (arrows, tilde keys, modified chords). Unknown finals are
    /// consumed and dropped — they are reports or private modes this
    /// client has no reason to forward (a render-mode client answers its
    /// panes' queries through the emulators, never through the host).
    fn scan_csi(&mut self, data: &[u8]) -> EscapeScan {
        // SGR mouse: ESC [ < cb ; col ; row M/m.
        if data.len() > 2 && data[2] == b'<' {
            let mut i = 3;
            let mut fields: [u32; 3] = [0; 3];
            let mut field = 0usize;
            let release;
            while i < data.len() {
                match data[i] {
                    b';' => field = (field + 1).min(2),
                    b'0'..=b'9' => fields[field] = fields[field] * 10 + u32::from(data[i] - b'0'),
                    b'M' | b'm' => {
                        release = data[i] == b'm';
                        i += 1;
                        let mouse = SgrMouse {
                            cb: u8::try_from(fields[0]).unwrap_or(u8::MAX),
                            col: u16::try_from(fields[1]).unwrap_or(u16::MAX),
                            row: u16::try_from(fields[2]).unwrap_or(u16::MAX),
                            release,
                        };
                        return EscapeScan::Token(Token::Mouse(mouse), i);
                    }
                    _ => {
                        // Malformed: consume through the stray byte.
                        return EscapeScan::Token(Token::Bytes(Vec::new()), i + 1);
                    }
                }
                i += 1;
            }
            return EscapeScan::Incomplete(data.len());
        }

        // A key sequence: params (digits and `;`) then one final byte
        // 0x40..=0x7E.
        let mut i = 2;
        let mut params_end = 2;
        while i < data.len() && (data[i].is_ascii_digit() || data[i] == b';') {
            if data[i] != b';' {
                params_end = i + 1;
            }
            i += 1;
        }
        if i >= data.len() {
            return EscapeScan::Incomplete(data.len());
        }
        let final_byte = data[i];
        if !(0x40..=0x7E).contains(&final_byte) {
            // Not a well-formed CSI: consume the byte and move on.
            return EscapeScan::Token(Token::Bytes(Vec::new()), i + 1);
        }
        let param_text = &data[2..params_end];
        let (first, modifier) = split_params(param_text);
        let consumed = i + 1;
        match csi_key(final_byte, first, modifier) {
            Some(key) => EscapeScan::Token(Token::Key(key), consumed),
            None => EscapeScan::Token(Token::Bytes(Vec::new()), consumed),
        }
    }
}

enum EscapeScan {
    Token(Token, usize),
    Incomplete(usize),
}

/// `params` text (`"1;5"`, `""`, `"3"`) → (first param, modifier param).
/// tmux/xterm spelling: an empty or missing first param means 1.
fn split_params(params: &[u8]) -> (u32, u32) {
    let mut parts = params.split(|b| *b == b';');
    let first = parts
        .next()
        .and_then(|p| std::str::from_utf8(p).ok())
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    let modifier = parts
        .next()
        .and_then(|p| std::str::from_utf8(p).ok())
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);
    (first, modifier)
}

/// Map a CSI final byte (plus params) to a key event. The xterm modifier
/// parameter (`1;{m}`) uses the same numeric order as the crate's
/// [`modifiers`] (shift 1, alt 2, ctrl 4), and 1 (or absent) means none.
fn csi_key(final_byte: u8, first: u32, modifier: u32) -> Option<TermKeyEvent> {
    let mods = xterm_mods(modifier);
    let key = match (final_byte, first) {
        (b'A', _) => TermKey::Up,
        (b'B', _) => TermKey::Down,
        (b'C', _) => TermKey::Right,
        (b'D', _) => TermKey::Left,
        (b'H', _) => TermKey::Home,
        (b'F', _) => TermKey::End,
        (b'E', _) => TermKey::End, // xterm's numpad-5-as-End spelling
        (b'P', _) => TermKey::F1,
        (b'Q', _) => TermKey::F2,
        (b'R', _) => TermKey::F3,
        (b'S', _) => TermKey::F4,
        (b'~', n) => match n {
            1 | 7 => TermKey::Home,
            2 => TermKey::Insert,
            3 => TermKey::Delete,
            4 | 8 => TermKey::End,
            5 => TermKey::PageUp,
            6 => TermKey::PageDown,
            11 => TermKey::F1,
            12 => TermKey::F2,
            13 => TermKey::F3,
            14 => TermKey::F4,
            15 => TermKey::F5,
            17 => TermKey::F6,
            18 => TermKey::F7,
            19 => TermKey::F8,
            20 => TermKey::F9,
            21 => TermKey::F10,
            23 => TermKey::F11,
            24 => TermKey::F12,
            _ => return None,
        },
        _ => return None,
    };
    Some(TermKeyEvent::functional(key, mods))
}

/// SS3 (`ESC O <byte>`) final byte → key, unmodified.
fn ss3_key(byte: u8, _mods: u8) -> Option<TermKey> {
    let key = match byte {
        b'A' => TermKey::Up,
        b'B' => TermKey::Down,
        b'C' => TermKey::Right,
        b'D' => TermKey::Left,
        b'H' => TermKey::Home,
        b'F' => TermKey::End,
        b'P' => TermKey::F1,
        b'Q' => TermKey::F2,
        b'R' => TermKey::F3,
        b'S' => TermKey::F4,
        _ => return None,
    };
    Some(key)
}

/// xterm modifier parameter → crate modifier bits. Identical values; a
/// parameter of 1 (or absent) means none.
fn xterm_mods(modifier: u32) -> u8 {
    let m = modifier.saturating_sub(1) & 0x7;
    m as u8
}

/// Re-encode one key event against the focused pane's tracked input state
/// (DECCKM application cursor, kitty keyboard flags, modifyOtherKeys) and
/// return the bytes to forward. Empty output means no encoding.
pub fn reencode_key(ev: &TermKeyEvent, pane: &Terminal) -> Vec<u8> {
    crate::keyboard::encode_key(ev, pane)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_str(parser: &mut InputParser, s: &str) -> Vec<Token> {
        parser.feed(s.as_bytes())
    }

    fn key_of(tokens: &[Token]) -> TermKey {
        match &tokens[0] {
            Token::Key(ev) => ev.key(),
            other => panic!("expected a Key token, got {other:?}"),
        }
    }

    #[test]
    fn plain_bytes_arrive_as_one_run() {
        let mut p = InputParser::default();
        assert_eq!(
            p.feed(b"hello \x03world"),
            vec![Token::Bytes(b"hello \x03world".to_vec())]
        );
    }

    #[test]
    fn csi_arrows_decode_unmodified() {
        let mut p = InputParser::default();
        let tokens = feed_str(&mut p, "\x1b[A\x1b[B\x1b[C\x1b[D");
        let keys: Vec<TermKey> = tokens
            .iter()
            .map(|t| match t {
                Token::Key(ev) => ev.key(),
                other => panic!("key expected, got {other:?}"),
            })
            .collect();
        assert_eq!(
            keys,
            vec![TermKey::Up, TermKey::Down, TermKey::Right, TermKey::Left]
        );
        for t in &tokens {
            if let Token::Key(ev) = t {
                assert_eq!(ev.modifiers, 0);
            }
        }
    }

    #[test]
    fn csi_modifier_params_decode() {
        // Ctrl+Right: ESC[1;5C — xterm modifier 5 = 1 + ctrl(4).
        let mut p = InputParser::default();
        let tokens = feed_str(&mut p, "\x1b[1;5C");
        assert_eq!(key_of(&tokens), TermKey::Right);
        if let Token::Key(ev) = &tokens[0] {
            assert_eq!(ev.modifiers, modifiers::CTRL);
        }
        // Shift+Tab is not a key this parser maps specially; BTab arrives
        // as ESC[Z which maps to nothing and is consumed.
        let tokens = feed_str(&mut p, "\x1b[Z");
        assert!(matches!(tokens[0], Token::Bytes(_)));
    }

    #[test]
    fn tilde_keys_decode() {
        let mut p = InputParser::default();
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[3~")), TermKey::Delete);
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[5~")), TermKey::PageUp);
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[6~")), TermKey::PageDown);
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[7~")), TermKey::Home);
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[15~")), TermKey::F5);
        assert_eq!(key_of(&feed_str(&mut p, "\x1b[24~")), TermKey::F12);
    }

    #[test]
    fn ss3_sequences_decode() {
        let mut p = InputParser::default();
        let tokens = feed_str(&mut p, "\x1bOP\x1bOA");
        assert_eq!(key_of(&tokens), TermKey::F1);
        assert_eq!(key_of(&tokens[1..]), TermKey::Up);
    }

    #[test]
    fn lone_escape_is_the_escape_key() {
        let mut p = InputParser::default();
        assert_eq!(key_of(&feed_str(&mut p, "\x1b")), TermKey::Escape);
        // ESC ESC: one Escape key then a fresh sequence position.
        let tokens = feed_str(&mut p, "\x1b\x1b[A");
        assert_eq!(key_of(&tokens), TermKey::Escape);
        assert_eq!(key_of(&tokens[1..]), TermKey::Up);
    }

    #[test]
    fn sequence_split_across_feeds_completes() {
        let mut p = InputParser::default();
        assert!(p.feed(b"\x1b[1;").is_empty(), "partial holds");
        let tokens = p.feed(b"5D");
        assert_eq!(key_of(&tokens), TermKey::Left);
        if let Token::Key(ev) = &tokens[0] {
            assert_eq!(ev.modifiers, modifiers::CTRL);
        }
    }

    #[test]
    fn sgr_mouse_reports_decode() {
        let mut p = InputParser::default();
        // Left press at col 11 row 6 (1-based).
        let tokens = feed_str(&mut p, "\x1b[<0;11;6M");
        match &tokens[0] {
            Token::Mouse(m) => {
                assert_eq!(m.cb, 0);
                assert_eq!(m.col, 11);
                assert_eq!(m.row, 6);
                assert!(!m.release);
                assert!(!m.is_motion());
            }
            other => panic!("mouse expected, got {other:?}"),
        }
        // Release.
        let tokens = feed_str(&mut p, "\x1b[<0;11;6m");
        match &tokens[0] {
            Token::Mouse(m) => assert!(m.release),
            other => panic!("mouse expected, got {other:?}"),
        }
        // Wheel up (cb 64) and drag (motion bit 32).
        let tokens = feed_str(&mut p, "\x1b[<64;3;4M");
        match &tokens[0] {
            Token::Mouse(m) => {
                assert!(m.is_wheel_up());
                assert!(!m.is_wheel_down());
            }
            other => panic!("mouse expected, got {other:?}"),
        }
        let tokens = feed_str(&mut p, "\x1b[<32;3;4M");
        match &tokens[0] {
            Token::Mouse(m) => {
                assert!(m.is_motion());
                assert_eq!(m.button(), 0);
            }
            other => panic!("mouse expected, got {other:?}"),
        }
    }

    #[test]
    fn mouse_reencode_rebases_coordinates() {
        // Host click at (11, 6) 1-based, pane origin (10, 0): pane-relative
        // (1, 6) 0-based → wire `ESC[<0;2;7M`.
        let m = SgrMouse {
            cb: 0,
            col: 11,
            row: 6,
            release: false,
        };
        assert_eq!(m.reencode_sgr(1, 6), b"\x1b[<0;2;7M");
        let m = SgrMouse {
            cb: 0,
            col: 11,
            row: 6,
            release: true,
        };
        assert_eq!(m.reencode_sgr(1, 6), b"\x1b[<0;2;7m");
    }

    #[test]
    fn reencode_follows_the_panes_decckm_mode() {
        // A plain terminal: Up is CSI A.
        let mut term = Terminal::new(80, 24);
        assert_eq!(
            reencode_key(&TermKeyEvent::functional(TermKey::Up, 0), &term),
            b"\x1b[A"
        );
        // The pane enables DECCKM (what a replay or %output carries).
        term.process(b"\x1b[?1h");
        assert!(term.application_cursor());
        assert_eq!(
            reencode_key(&TermKeyEvent::functional(TermKey::Up, 0), &term),
            b"\x1bOA"
        );
        // …and Home likewise.
        assert_eq!(
            reencode_key(&TermKeyEvent::functional(TermKey::Home, 0), &term),
            b"\x1bOH"
        );
    }
}
