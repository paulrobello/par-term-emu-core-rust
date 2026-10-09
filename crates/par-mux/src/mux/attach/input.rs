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
//!   against the focused pane's terminal (`par_term_emu_core::keyboard::encode_key`
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
//!
//! Bracketed paste (`ESC[200~` … `ESC[201~`, which a host left in DECSET
//! 2004 mode by an earlier app sends around every paste) is emitted as one
//! [`Token::Paste`]: the body passes through verbatim — embedded prefix
//! bytes and partial escape fragments included — because it is text the
//! user meant to insert, not keystrokes to decode. The markers are
//! consumed here; the router re-frames the body for a pane that asked for
//! bracketed paste.

use par_term_emu_core::keyboard::{modifiers, TermKey, TermKeyEvent};
use par_term_emu_core::terminal::Terminal;

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
    /// A bracketed-paste body: forwarded to the focused pane opaque, never
    /// scanned for chords. The `ESC[200~`/`ESC[201~` markers are not part
    /// of it.
    Paste(Vec<u8>),
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
    #[allow(dead_code)] // unreachable since ARC-134 narrowed attach to pub(crate); cleanup candidate
    pub fn button(&self) -> u8 {
        self.cb & 0x3
    }

    /// A right-button PRESS (not release/motion/wheel): the context
    /// menus' trigger. Ghostty spells it `ESC[<2;col;rowM` — button 2,
    /// no motion bit, `M` terminator.
    pub fn is_right_press(&self) -> bool {
        !self.release && !self.is_motion() && self.cb & !0x3 & 0x40 == 0 && self.cb & 0x3 == 2
    }

    /// The modifier bits, in the crate's [`modifiers`] order — the SGR
    /// wire puts them at shift 2 with the same bit values (shift 1, alt 2,
    /// ctrl 4).
    #[allow(dead_code)] // unreachable since ARC-134 narrowed attach to pub(crate); cleanup candidate
    pub fn modifiers(&self) -> u8 {
        (self.cb >> 2) & 0x7
    }

    /// Re-encode as a pane-relative SGR sequence: coordinates rebased to
    /// the pane's own 0-based origin. Always SGR, per tmux's
    /// re-encode-to-the-pane behavior with the modern encoding every mouse
    /// app negotiates (legacy pane encodings are a documented
    /// simplification).
    pub fn reencode_sgr(&self, rel_col: u16, rel_row: u16) -> Vec<u8> {
        let event = par_term_emu_core::mouse::MouseEvent::new(
            self.cb,
            rel_col as usize,
            rel_row as usize,
            !self.release,
            0,
        );
        event.encode(
            par_term_emu_core::mouse::MouseMode::AnyEvent,
            par_term_emu_core::mouse::MouseEncoding::Sgr,
        )
    }
}

/// The bracketed-paste terminator the opaque body scan hunts for.
const PASTE_END: &[u8] = b"\x1b[201~";
/// A paste opened by `ESC[200~` stops holding its body after this many
/// bytes: the held bytes stream out as a `Token::Paste` chunk and the
/// parser stays in paste mode until the terminator, so memory stays bounded
/// and no byte of a huge clipboard reaches the chord scanner.
const PASTE_HELD_CAP: usize = 1 << 20;

/// Incremental stdin tokenizer: feed raw bytes, take complete tokens. A
/// partial escape sequence at the end of a burst stays pending until the
/// next feed completes it; a lone ESC (no following byte this burst) is
/// emitted as the Escape key at once — the classic interactivity trade,
/// and what xterm does with no timeout configured. The same trade applies
/// to a paste whose `ESC[200~` opener arrives as its own lone-ESC burst.
#[derive(Default)]
pub struct InputParser {
    pending: Vec<u8>,
    /// Inside a bracketed paste: everything until `PASTE_END` is opaque.
    paste: bool,
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
            if self.paste {
                // Opaque scan for the terminator; the body is emitted
                // verbatim as one Paste token. Held-byte accounting: the
                // tail either stays pending (terminator may still arrive)
                // or flushes as output at the cap — never both.
                match find_subslice(&data[i..], PASTE_END) {
                    Some(rel) => {
                        if rel > 0 {
                            tokens.push(Token::Paste(data[i..i + rel].to_vec()));
                        }
                        i += rel + PASTE_END.len();
                        self.paste = false;
                    }
                    None => {
                        if data.len() - i > PASTE_HELD_CAP {
                            // Stop holding, but stay in paste mode: the rest
                            // of a huge clipboard must never reach the chord
                            // scanner. Keep a terminator-sized tail pending
                            // so a terminator split across this flush is
                            // still recognized.
                            let flush_end = data.len() - (PASTE_END.len() - 1);
                            tokens.push(Token::Paste(data[i..flush_end].to_vec()));
                            self.pending = data[flush_end..].to_vec();
                        } else {
                            self.pending = data[i..].to_vec();
                        }
                        i = data.len();
                    }
                }
                continue;
            }
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
            // An OSC 11 color-report reply the probe's 150 ms window
            // missed: a host slower than the deadline answers while the
            // pump owns stdin. Drop the report whole (BEL or ST
            // terminator, bounded so a malformed report cannot hold the
            // input stream) — fragments of it must never forward to a
            // pane. A human Alt+] followed by the literal "11;" is not a
            // real input shape.
            b']' if data.len() >= 5 && &data[2..5] == b"11;" => {
                let mut i = 5;
                while i < data.len() {
                    match data[i] {
                        0x07 => return EscapeScan::Token(Token::Bytes(Vec::new()), i + 1),
                        0x1b if i + 1 < data.len() && data[i + 1] == b'\\' => {
                            return EscapeScan::Token(Token::Bytes(Vec::new()), i + 2)
                        }
                        // A stray ESC without the ST partner ends the
                        // malformed report; consume through it.
                        0x1b => return EscapeScan::Token(Token::Bytes(Vec::new()), i + 1),
                        _ => i += 1,
                    }
                }
                if data.len() > 64 {
                    // Give up on a report that never terminates; drop the
                    // prefix so the stream keeps flowing.
                    return EscapeScan::Token(Token::Bytes(Vec::new()), data.len());
                }
                EscapeScan::Incomplete(data.len())
            }
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
        // Bracketed paste start (the host is in DECSET 2004 mode left by
        // an earlier app): switch the parser into the opaque-body scan.
        // The markers themselves are consumed — the pane gets the pasted
        // text, not the framing.
        if final_byte == b'~' && param_text == b"200" {
            self.paste = true;
            return EscapeScan::Token(Token::Bytes(Vec::new()), i + 1);
        }
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

/// First index of `needle` in `haystack`, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `body` with every `ESC[201~` removed, repeated until none remains (a
/// removal can splice a new terminator out of the surrounding bytes), so
/// pasted text cannot close the pane's bracketed paste early.
pub(crate) fn strip_paste_end(body: &[u8]) -> Vec<u8> {
    let mut out = body.to_vec();
    while let Some(at) = find_subslice(&out, PASTE_END) {
        out.drain(at..at + PASTE_END.len());
    }
    out
}

/// Re-encode one key event against the focused pane's tracked input state
/// (DECCKM application cursor, kitty keyboard flags, modifyOtherKeys) and
/// return the bytes to forward. Empty output means no encoding.
#[allow(dead_code)] // unreachable since ARC-134 narrowed attach to pub(crate); cleanup candidate
pub fn reencode_key(ev: &TermKeyEvent, pane: &Terminal) -> Vec<u8> {
    par_term_emu_core::keyboard::encode_key(ev, pane)
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

    /// The right-button press decodes: button 2, press terminator `M`,
    /// no motion bit; release, motion, wheels, and other buttons are
    /// not right presses — the context menus' trigger predicate.
    #[test]
    fn right_press_decodes_and_rejects_its_lookalikes() {
        let press = |cb: u8, release: bool| SgrMouse {
            cb,
            col: 7,
            row: 1,
            release,
        };
        // Ghostty's `ESC[<2;col;rowM` right-press spelling.
        assert!(press(2, false).is_right_press());
        assert!(!press(0, false).is_right_press(), "left button");
        assert!(!press(1, false).is_right_press(), "middle button");
        assert!(!press(2, true).is_right_press(), "release report");
        // Right DRAG: the motion bit rides button 2 (cb 34) — not a press.
        assert!(!press(34, false).is_right_press(), "right drag");
        // Wheel lookalike: wheel codes carry bit 6 (right wheel = 66).
        assert!(!press(66, false).is_right_press(), "right wheel");
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

    /// An OSC 11 color-report reply that arrived AFTER the probe's 150 ms
    /// window (the stdin reader owns the stream now) is dropped WHOLE -
    /// no fragment of it forwards to a pane. The graceful half of the
    /// probe-race contract.
    #[test]
    fn late_osc11_reply_is_dropped_not_forwarded() {
        let mut parser = InputParser::default();
        let tokens = parser.feed(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\");
        assert_eq!(tokens.len(), 1);
        assert!(matches!(tokens[0], Token::Bytes(ref b) if b.is_empty()));
        // The parser stays healthy for the next keystroke.
        let tokens = parser.feed(b"x");
        resync_health_check(&tokens);
    }

    /// Hostile-input helpers shared by the adversarial suite below.
    ///
    /// The parser's one documented divergence between incremental and
    /// whole-stream decoding: a burst that ends on a bare ESC emits the
    /// Escape key immediately (xterm's no-timeout trade). Every other split
    /// of a stream must decode exactly like the whole-stream feed.
    const PASTE_START: &[u8] = b"\x1b[200~";

    /// A paste body carrying exactly what decode-as-keystrokes would mangle:
    /// the prefix byte (C-b, 0x02), a complete CSI key sequence, a partial
    /// CSI fragment, and a bare ESC.
    fn hostile_paste_body() -> Vec<u8> {
        let mut body = b"echo hi".to_vec();
        body.extend_from_slice(&[0x02]);
        body.extend_from_slice(b"\x1b[3~");
        body.extend_from_slice(b"\x1b[1;5");
        body.push(0x1b);
        body.extend_from_slice(b" tail");
        body
    }

    /// The designed lone-ESC semantics of one-byte bursts: a single-byte feed
    /// can never complete a sequence, so ESC alone is the Escape key and any
    /// other byte is its own plain run.
    fn expected_byte_at_a_time(stream: &[u8]) -> Vec<Token> {
        stream
            .iter()
            .map(|&b| {
                if b == 0x1b {
                    Token::Key(TermKeyEvent::functional(TermKey::Escape, 0))
                } else {
                    Token::Bytes(vec![b])
                }
            })
            .collect()
    }

    /// Feed `stream` one byte per burst.
    fn feed_byte_at_a_time(stream: &[u8]) -> Vec<Token> {
        let mut parser = InputParser::default();
        let mut tokens = Vec::new();
        for b in stream {
            tokens.extend(parser.feed(std::slice::from_ref(b)));
        }
        tokens
    }

    /// Merge adjacent Bytes tokens so decodes that differ only in where feed
    /// boundaries flushed plain runs compare equal.
    fn merge_adjacent_bytes(tokens: Vec<Token>) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::new();
        for token in tokens {
            match (out.last_mut(), token) {
                (Some(Token::Bytes(last)), Token::Bytes(b)) => last.extend_from_slice(&b),
                (_, other) => out.push(other),
            }
        }
        out
    }

    /// Concatenate every Bytes and Paste token: the byte-accounting view of
    /// a decode.
    fn bytes_of(tokens: &[Token]) -> Vec<u8> {
        let mut out = Vec::new();
        for token in tokens {
            if let Token::Bytes(b) | Token::Paste(b) = token {
                out.extend_from_slice(b);
            }
        }
        out
    }

    /// The stream decodes byte-at-a-time exactly per the lone-ESC design, and
    /// — when it carries no ESC at all — exactly like the whole-stream feed.
    fn assert_byte_at_a_time_contract(stream: &[u8]) {
        assert_eq!(
            feed_byte_at_a_time(stream),
            expected_byte_at_a_time(stream),
            "byte-at-a-time decode diverged from the lone-ESC design for {stream:?}"
        );
        if !stream.contains(&0x1b) {
            assert_eq!(
                bytes_of(&feed_byte_at_a_time(stream)),
                bytes_of(&InputParser::default().feed(stream)),
                "a no-ESC stream must decode to identical bytes byte-at-a-time: {stream:?}"
            );
        }
    }

    /// Assert a decode is exactly one Paste token equal to `body` and no
    /// non-empty plain run (empty Bytes tokens are the established drop
    /// shape).
    fn assert_single_paste(tokens: &[Token], body: &[u8]) {
        let pastes: Vec<&Vec<u8>> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Paste(b) => Some(b),
                _ => None,
            })
            .collect();
        assert_eq!(pastes.len(), 1, "expected one paste token, got {tokens:?}");
        assert_eq!(pastes[0], &body, "paste body must pass through untouched");
        assert!(
            !tokens
                .iter()
                .any(|t| matches!(t, Token::Bytes(b) if !b.is_empty())),
            "no paste byte may leak as a plain run: {tokens:?}"
        );
    }

    /// After any hostile input the parser must still decode a fresh plain
    /// keystroke normally — no wedged state.
    fn resync_health_check(tokens: &[Token]) {
        assert!(matches!(
            tokens[0],
            Token::Bytes(ref b) if b == b"x"
        ));
    }

    #[test]
    fn paste_pair_decodes_to_one_opaque_run() {
        let body = hostile_paste_body();
        let mut stream = PASTE_START.to_vec();
        stream.extend_from_slice(&body);
        stream.extend_from_slice(PASTE_END);
        let mut parser = InputParser::default();
        let tokens = parser.feed(&stream);
        assert_single_paste(&tokens, &body);
        // Trailing input after the paste decodes normally.
        let tokens = parser.feed(b"x");
        resync_health_check(&tokens);
    }

    #[test]
    fn paste_body_split_across_feeds_reassembles_untouched() {
        let body = hostile_paste_body();
        let mut stream = PASTE_START.to_vec();
        stream.extend_from_slice(&body);
        stream.extend_from_slice(PASTE_END);

        // Burst every 3 bytes: the marker completes in the first burst, the
        // terminator may split anywhere. Concatenated Bytes output must equal
        // the body byte-for-byte (no loss, no mangling).
        let mut parser = InputParser::default();
        let mut reassembled = Vec::new();
        for chunk in stream.chunks(3) {
            for token in parser.feed(chunk) {
                match token {
                    Token::Paste(b) => reassembled.extend_from_slice(&b),
                    Token::Bytes(b) => assert!(b.is_empty(), "paste byte leaked as plain"),
                    other => panic!("unexpected token inside a paste: {other:?}"),
                }
            }
        }
        assert_eq!(reassembled, body, "byte loss or mangling across feeds");
    }

    /// (a) One byte per burst: the decode follows the lone-ESC design
    /// exactly, and streams without any ESC match the whole-stream feed.
    #[test]
    fn byte_at_a_time_decode_follows_the_design() {
        assert_byte_at_a_time_contract(b"hello \x03world");
        assert_byte_at_a_time_contract(&[0x02]); // the prefix chord byte
        assert_byte_at_a_time_contract(b"\x1b[1;5C");
        let mut stream = PASTE_START.to_vec();
        stream.extend_from_slice(&hostile_paste_body());
        stream.extend_from_slice(PASTE_END);
        assert_byte_at_a_time_contract(&stream);
    }

    /// (b) Split reads at every byte boundary of the paste marker. Every
    /// split except a burst ending on the bare ESC completes the marker and
    /// decodes the body opaquely; the bare-ESC split is the documented
    /// lone-ESC divergence and its exact shape is pinned here.
    #[test]
    fn paste_marker_splits_at_every_byte_boundary() {
        let body = b"payload".to_vec();
        for k in 1..=PASTE_START.len() {
            let mut parser = InputParser::default();
            let first = parser.feed(&PASTE_START[..k]);
            let mut rest = PASTE_START[k..].to_vec();
            rest.extend_from_slice(&body);
            rest.extend_from_slice(PASTE_END);
            let second = parser.feed(&rest);
            if k == 1 {
                // Lone ESC: the Escape key fires, the rest of the marker is
                // plain, and the stream still flows (terminator CSI dropped,
                // body unbracketed but intact).
                assert_eq!(
                    first,
                    vec![Token::Key(TermKeyEvent::functional(TermKey::Escape, 0))]
                );
                assert_eq!(bytes_of(&second), b"[200~payload");
            } else if k == PASTE_START.len() {
                // The complete opener as its own burst: the empty
                // drop-shape token, paste mode armed.
                assert_eq!(first, vec![Token::Bytes(Vec::new())], "split {k}");
                assert_single_paste(&second, &body);
            } else {
                assert!(first.is_empty(), "split {k} held: {first:?}");
                assert_single_paste(&second, &body);
            }
        }
    }

    /// (d) Bursts interleaving a paste, a CSI chord key, and plain runs in
    /// one feed — the decoded token order matches the wire order.
    #[test]
    fn burst_interleaves_paste_chord_and_plain_runs() {
        let mut stream = b"before ".to_vec();
        stream.extend_from_slice(PASTE_START);
        let body = hostile_paste_body();
        stream.extend_from_slice(&body);
        stream.extend_from_slice(PASTE_END);
        stream.extend_from_slice(b"\x1b[1;5C");
        stream.extend_from_slice(&[0x02]);
        stream.extend_from_slice(b" after");
        let mut parser = InputParser::default();
        let tokens = parser.feed(&stream);
        // The marker contributes an empty Bytes token (the established drop
        // shape), so: [Bytes("before "), Bytes(empty), Paste(body),
        // Key(Right, ctrl), Bytes(0x02 + " after")].
        assert_eq!(tokens.len(), 5, "{tokens:?}");
        assert!(matches!(tokens[0], Token::Bytes(ref b) if b == b"before "));
        assert!(matches!(tokens[1], Token::Bytes(ref b) if b.is_empty()));
        assert_single_paste(&tokens[1..3], &body);
        assert!(
            matches!(tokens[3], Token::Key(ref ev) if ev.key() == TermKey::Right && ev.modifiers == modifiers::CTRL)
        );
        assert!(
            matches!(tokens[4], Token::Bytes(ref b) if b.len() == 7 && b[0] == 0x02 && &b[1..] == b" after")
        );

        // The same stream in three bursts decodes to the same tokens.
        let mut parser = InputParser::default();
        let mut bursted = Vec::new();
        for chunk in [
            &stream[..7],
            &stream[7..stream.len() - 6],
            &stream[stream.len() - 6..],
        ] {
            bursted.extend(parser.feed(chunk));
        }
        assert_eq!(merge_adjacent_bytes(bursted), merge_adjacent_bytes(tokens));
    }

    /// A paste larger than the hold cap streams out as successive Paste
    /// chunks and STAYS in paste mode: a chord byte and an escape sequence
    /// past the cap are still paste text, every byte arrives as Paste, and
    /// the terminator (even split across the flush) ends the paste.
    #[test]
    fn paste_past_the_cap_streams_and_stays_opaque() {
        let mut parser = InputParser::default();
        let mut tokens = parser.feed(PASTE_START);
        let mut body = vec![b'x'; PASTE_HELD_CAP + 10];
        body.extend_from_slice(&[0x02, b'x']); // prefix + kill-pane chord
        body.extend_from_slice(b"\x1b[3~"); // a key sequence
        body.extend_from_slice(&vec![b'y'; PASTE_HELD_CAP]);
        let mut stream = body.clone();
        stream.extend_from_slice(PASTE_END);
        for chunk in stream.chunks(PASTE_HELD_CAP / 3) {
            tokens.extend(parser.feed(chunk));
        }
        let mut pasted = Vec::new();
        for token in &tokens {
            match token {
                Token::Paste(b) => pasted.extend_from_slice(b),
                Token::Bytes(b) => assert!(b.is_empty(), "paste bytes leaked as plain"),
                other => panic!("non-paste token inside a paste: {other:?}"),
            }
        }
        assert_eq!(pasted, body, "every body byte arrives as Paste, in order");
        // The terminator ended the paste: the parser is healthy.
        resync_health_check(&parser.feed(b"x"));
    }

    /// An unterminated paste stops holding at the cap — the held body
    /// streams out as Paste — but stays in paste mode until its terminator.
    #[test]
    fn unterminated_paste_flushes_at_the_cap() {
        let mut parser = InputParser::default();
        // The opener alone: consumed, paste mode on, empty drop-shape token.
        assert_eq!(parser.feed(PASTE_START), vec![Token::Bytes(Vec::new())]);
        // Under the cap: held pending, nothing decoded.
        let chunk = vec![b'x'; PASTE_HELD_CAP / 2];
        assert!(parser.feed(&chunk).is_empty());
        assert!(parser.feed(&chunk).is_empty());
        // Crossing the cap flushes the held body (less a terminator-sized
        // tail) as Paste and stays in paste mode.
        let tokens = parser.feed(&chunk);
        assert!(
            matches!(tokens.as_slice(), [Token::Paste(_)]),
            "{}",
            tokens.len()
        );
        assert_eq!(
            bytes_of(&tokens).len(),
            PASTE_HELD_CAP + PASTE_HELD_CAP / 2 - (PASTE_END.len() - 1),
            "the held body flushed"
        );
        // Still pasting: a chord byte is held as body, not decoded.
        assert!(parser.feed(&[0x02]).is_empty());
        // The terminator ends it; the tail arrives as Paste.
        let tokens = parser.feed(PASTE_END);
        assert_eq!(bytes_of(&tokens), b"xxxxx\x02");
        assert!(tokens.iter().all(|t| matches!(t, Token::Paste(_))));
        resync_health_check(&parser.feed(b"x"));
    }
}
