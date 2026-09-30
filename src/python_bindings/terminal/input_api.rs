//! Key-input encoding API for `PyTerminal` (ENH-028): exposes the shared
//! encoder (legacy xterm, modifyOtherKeys, option-key modes, kitty level 1)
//! so Python frontends produce the same bytes as par-term and the C FFI.

use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    /// Encode a key event into the bytes a terminal application expects.
    ///
    /// The encoding honors the terminal's negotiated input state —
    /// application cursor keys, kitty keyboard flags, and the
    /// modifyOtherKeys mode the running program requested.
    ///
    /// Args:
    ///     key: Key code — a ``TERM_KEY_*``-style value: 1 for a character
    ///         key, or a functional-key code (9 Tab, 13 Enter, 27 Escape,
    ///         127 Backspace, 57428 Left … 57437 End, 57376..57387 F1..F12).
    ///     modifiers: Bitfield — shift=1, alt=2, ctrl=4, super=8, hyper=16,
    ///         meta=32, plus the side-info bit alt_right=64 (the held Alt
    ///         key is the right one, selecting ``right_option``).
    ///     codepoint: Unicode scalar for character keys (``key=1``): the
    ///         typed form for plain text, the base form when ctrl/alt drive
    ///         the encoding, the OS-composed glyph in normal option mode.
    ///         Ignored for functional keys.
    ///     left_option: Option-key mode for the left Alt key —
    ///         0 normal (pass the character through), 1 meta (8th bit on
    ///         ASCII bases), 2 esc (ESC-prefix). Default 2 (esc) — the same
    ///         default as C ``ptec_terminal_encode_key`` and Rust
    ///         ``KeyEncodeOptions::default()``.
    ///     right_option: Option-key mode for the right Alt key. Default 2
    ///         (esc).
    ///
    /// Returns:
    ///     bytes: The bytes to write to the PTY. Empty bytes mean the key
    ///     has no encoding (unknown key code).
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.encode_key(1, 4, ord("c"))   # b'\\x03'  (Ctrl+C)
    ///     term.encode_key(57430, 0, 0)      # b'\\x1b[A' (Up arrow)
    ///     term.encode_key(1, 2, ord("f"))   # b'\\x1bf'  (Alt+f, esc default)
    ///     term.encode_key(1, 2, ord("f"), left_option=0)  # b'f'
    ///     ```
    #[pyo3(signature = (key, modifiers, codepoint = 0, left_option = 2, right_option = 2))]
    fn encode_key(
        &self,
        key: u16,
        modifiers: u8,
        codepoint: u32,
        left_option: u8,
        right_option: u8,
    ) -> Vec<u8> {
        let ev = crate::keyboard::TermKeyEvent {
            key,
            modifiers,
            _pad: 0,
            codepoint,
        };
        let opts = crate::keyboard::KeyEncodeOptions {
            left_option,
            right_option,
        };
        crate::keyboard::encode_key_with(&ev, &self.inner, &opts)
    }
}
