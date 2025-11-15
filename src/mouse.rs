/// Mouse tracking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    /// No mouse tracking
    Off,
    /// X10 mode - press events only
    X10,
    /// Normal mode - press and release
    Normal,
    /// Button event mode - press, release, and motion while button pressed
    ButtonEvent,
    /// Any event mode - all mouse motion
    AnyEvent,
}

/// Mouse encoding format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEncoding {
    /// Default X11 encoding
    Default,
    /// UTF-8 encoding
    Utf8,
    /// SGR encoding (1006)
    Sgr,
    /// URXVT encoding (1015)
    Urxvt,
}

/// Mouse event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: u8,
    pub col: usize,
    pub row: usize,
    pub pressed: bool,
    pub modifiers: u8,
}

impl MouseEvent {
    /// Create a new mouse event
    pub fn new(button: u8, col: usize, row: usize, pressed: bool, modifiers: u8) -> Self {
        Self {
            button,
            col,
            row,
            pressed,
            modifiers,
        }
    }

    /// Encode mouse event to bytes based on encoding format
    pub fn encode(&self, mode: MouseMode, encoding: MouseEncoding) -> Vec<u8> {
        match encoding {
            MouseEncoding::Sgr => self.encode_sgr(mode),
            MouseEncoding::Urxvt => self.encode_urxvt(),
            MouseEncoding::Utf8 => self.encode_utf8(),
            MouseEncoding::Default => self.encode_default(),
        }
    }

    fn encode_sgr(&self, _mode: MouseMode) -> Vec<u8> {
        let button_code = self.button | (self.modifiers << 2);
        let release = if self.pressed { 'M' } else { 'm' };
        format!(
            "\x1b[<{};{};{}{}",
            button_code,
            self.col + 1,
            self.row + 1,
            release
        )
        .into_bytes()
    }

    fn encode_urxvt(&self) -> Vec<u8> {
        let button_code = self.button | (self.modifiers << 2) | if self.pressed { 0 } else { 3 };
        format!(
            "\x1b[{};{};{}M",
            button_code + 32,
            self.col + 1,
            self.row + 1
        )
        .into_bytes()
    }

    fn encode_utf8(&self) -> Vec<u8> {
        let button_code = self.button | (self.modifiers << 2) | if self.pressed { 0 } else { 3 };
        let mut bytes = vec![b'\x1b', b'[', b'M', button_code + 32];
        bytes.extend(&[(self.col + 1) as u8 + 32, (self.row + 1) as u8 + 32]);
        bytes
    }

    fn encode_default(&self) -> Vec<u8> {
        let button_code = self.button | (self.modifiers << 2) | if self.pressed { 0 } else { 3 };
        vec![
            b'\x1b',
            b'[',
            b'M',
            button_code + 32,
            (self.col + 1).min(223) as u8 + 32,
            (self.row + 1).min(223) as u8 + 32,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mouse_event_sgr() {
        let event = MouseEvent::new(0, 10, 5, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        let expected = b"\x1b[<0;11;6M";
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_mouse_event_release() {
        let event = MouseEvent::new(0, 10, 5, false, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        let expected = b"\x1b[<0;11;6m";
        assert_eq!(encoded, expected);
    }
}
