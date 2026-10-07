//! `send-keys` payload grammar: key names, `-l` literals, `-H` hex bytes.

use super::*;

/// Map one send-keys token to its payload part (tmux key names,
/// `key-string.c`).
///
/// Two classes, on purpose:
/// - The control-byte names par-term's `escape_keys_for_tmux` emits for
///   bytes it has ALREADY encoded (`C-x`, `C-Space`, `Escape`, `BSpace`,
///   `Space`), plus `Enter` and `Tab`, stay raw bytes. Re-encoding them
///   against the pane's modes would double-encode par-term input (`C-c`
///   under modifyOtherKeys 2 would become `CSI 27;5;99~`).
/// - Navigation and function keys, which par-term never emits by name,
///   resolve to a key event that dispatch encodes against the pane's
///   DECCKM/kitty state, as tmux does (`input-keys.c`).
///
/// Unknown tokens are NOT errors: they are written literally, so
/// passthrough text works without quoting every word (a deliberate,
/// narrower contract than tmux's, which rejects unknown key names).
pub(super) fn key_part(name: &str) -> Option<SendKeysPart> {
    let key = |key: TermKey| Some(SendKeysPart::Key(TermKeyEvent::functional(key, 0)));
    let byte = |b: u8| Some(SendKeysPart::Bytes(vec![b]));
    match name {
        "C-Space" => byte(0x00),
        "Enter" => byte(0x0d),
        "Tab" => byte(0x09),
        "Escape" | "Esc" => byte(0x1b),
        "BSpace" => byte(0x7f),
        "Space" => byte(b' '),
        "Up" => key(TermKey::Up),
        "Down" => key(TermKey::Down),
        "Right" => key(TermKey::Right),
        "Left" => key(TermKey::Left),
        "Home" => key(TermKey::Home),
        "End" => key(TermKey::End),
        "PageUp" | "PgUp" | "PPage" => key(TermKey::PageUp),
        "PageDown" | "PgDn" | "NPage" => key(TermKey::PageDown),
        "IC" | "Insert" => key(TermKey::Insert),
        "DC" | "Delete" => key(TermKey::Delete),
        "F1" => key(TermKey::F1),
        "F2" => key(TermKey::F2),
        "F3" => key(TermKey::F3),
        "F4" => key(TermKey::F4),
        "F5" => key(TermKey::F5),
        "F6" => key(TermKey::F6),
        "F7" => key(TermKey::F7),
        "F8" => key(TermKey::F8),
        "F9" => key(TermKey::F9),
        "F10" => key(TermKey::F10),
        "F11" => key(TermKey::F11),
        "F12" => key(TermKey::F12),
        "BTab" => Some(SendKeysPart::Key(TermKeyEvent::functional(
            TermKey::Tab,
            modifiers::SHIFT,
        ))),
        _ => {
            let letter = name.strip_prefix("C-")?;
            match *letter.as_bytes() {
                [c] if c.is_ascii_alphabetic() => byte(c.to_ascii_lowercase() - b'a' + 1),
                _ => None,
            }
        }
    }
}

/// A bare `0xNN` token: one raw byte, the form `escape_keys_for_tmux` uses
/// for high bytes.
pub(super) fn hex_byte_token(token: &str) -> Option<u8> {
    let digits = token.strip_prefix("0x")?;
    if digits.len() != 2 {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

/// Parse a send-keys payload into its parts (see [`key_part`]).
///
/// Three modes, mirroring tmux's contract:
/// - default: tokens are keys — quoted or bare words resolve through the key
///   table, `0xNN` tokens are raw bytes, anything else is literal text.
///   Tokens join with NOTHING between them; a space must be an explicit
///   `Space` key or live inside a quoted run (exactly how
///   `escape_keys_for_tmux` encodes spaces).
/// - `-l`: everything is literal text (quotes still resolved, no key
///   interpretation). Tokens also join with NOTHING between them — measured
///   on tmux 3.7c, `send-keys -l echo LEFT` types `echoLEFT`, so a space
///   must live inside a quoted run here too.
/// - `-H`: tokens are hex byte pairs, `0x` prefix optional.
///
/// No terminator is appended in any mode.
pub(super) fn parse_send_keys_payload(raw: &str) -> Result<SendKeysPayload, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let (literal, hex, body) = match raw {
        "-l" | "-H" => return Err("send-keys requires a payload".to_string()),
        _ if let Some(rest) = raw.strip_prefix("-l ") => (true, false, rest),
        _ if let Some(rest) = raw.strip_prefix("-H ") => (false, true, rest),
        _ => (false, false, raw),
    };
    let tokens = shell_split(body);
    if tokens.is_empty() {
        return Err("send-keys requires a payload".to_string());
    }
    let mut out = SendKeysPayload::default();
    if hex {
        for token in &tokens {
            let digits = token.strip_prefix("0x").unwrap_or(token);
            let byte =
                u8::from_str_radix(digits, 16).map_err(|_| format!("invalid hex byte: {token}"))?;
            out.push_bytes(&[byte]);
        }
    } else if literal {
        for token in tokens {
            out.push_bytes(token.as_bytes());
        }
    } else {
        for token in tokens {
            if let Some(part) = key_part(&token) {
                out.push(part);
            } else if let Some(byte) = hex_byte_token(&token) {
                out.push_bytes(&[byte]);
            } else {
                out.push_bytes(token.as_bytes());
            }
        }
    }
    Ok(out)
}
