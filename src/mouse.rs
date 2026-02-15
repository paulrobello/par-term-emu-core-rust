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

/// Mouse event type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventType {
    /// Mouse button press
    Press,
    /// Mouse button release
    Release,
    /// Mouse movement (with or without button held)
    Move,
    /// Mouse drag (move with button held)
    Drag,
    /// Mouse scroll up
    ScrollUp,
    /// Mouse scroll down
    ScrollDown,
}

/// Mouse button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
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

/// Mouse event record with position and metadata
#[derive(Debug, Clone)]
pub struct MouseEventRecord {
    /// Event type
    pub event_type: MouseEventType,
    /// Mouse button involved
    pub button: MouseButton,
    /// Column position (0-indexed)
    pub col: usize,
    /// Row position (0-indexed)
    pub row: usize,
    /// Pixel position (for SGR 1016)
    pub pixel_x: Option<u16>,
    pub pixel_y: Option<u16>,
    /// Modifier keys (shift, alt, ctrl)
    pub modifiers: u8,
    /// Timestamp in microseconds
    pub timestamp: u64,
}

/// Mouse position history entry
#[derive(Debug, Clone)]
pub struct MousePosition {
    pub col: usize,
    pub row: usize,
    pub timestamp: u64,
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 17: Advanced Mouse Support ===

    /// Record a mouse event in history
    pub fn record_mouse_event(
        &mut self,
        event_type: MouseEventType,
        button: MouseButton,
        col: usize,
        row: usize,
        modifiers: u8,
    ) {
        let record = MouseEventRecord {
            event_type,
            button,
            col,
            row,
            pixel_x: None, // Could be populated if cell size known
            pixel_y: None,
            modifiers,
            timestamp: crate::terminal::get_timestamp_us(),
        };

        self.mouse_events.push(record);
        if self.mouse_events.len() > self.max_mouse_history {
            self.mouse_events.remove(0);
        }

        // Also record position history
        self.mouse_positions.push(MousePosition {
            col,
            row,
            timestamp: crate::terminal::get_timestamp_us(),
        });
        if self.mouse_positions.len() > self.max_mouse_history {
            self.mouse_positions.remove(0);
        }
    }

    /// Get mouse event history
    pub fn get_mouse_history(&self) -> &[MouseEventRecord] {
        &self.mouse_events
    }

    /// Get recent mouse positions
    pub fn get_mouse_positions(&self) -> &[MousePosition] {
        &self.mouse_positions
    }

    /// Clear mouse history
    pub fn clear_mouse_history(&mut self) {
        self.mouse_events.clear();
        self.mouse_positions.clear();
    }

    /// Set the maximum number of mouse events to retain
    pub fn set_max_mouse_history(&mut self, max: usize) {
        self.max_mouse_history = max;
        if self.mouse_events.len() > max {
            self.mouse_events.drain(0..self.mouse_events.len() - max);
        }
        if self.mouse_positions.len() > max {
            self.mouse_positions
                .drain(0..self.mouse_positions.len() - max);
        }
    }

    /// Get the maximum number of mouse events to retain
    pub fn get_max_mouse_history(&self) -> usize {
        self.max_mouse_history
    }
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
        let col = self.col.saturating_add(1).min(223) as u8 + 32;
        let row = self.row.saturating_add(1).min(223) as u8 + 32;
        bytes.extend(&[col, row]);
        bytes
    }

    fn encode_default(&self) -> Vec<u8> {
        let button_code = self.button | (self.modifiers << 2) | if self.pressed { 0 } else { 3 };
        vec![
            b'\x1b',
            b'[',
            b'M',
            button_code + 32,
            self.col.saturating_add(1).min(223) as u8 + 32,
            self.row.saturating_add(1).min(223) as u8 + 32,
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

    #[test]
    fn test_mouse_event_default_encoding() {
        let event = MouseEvent::new(0, 10, 5, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Default);
        assert_eq!(encoded.len(), 6);
        assert_eq!(encoded[0], b'\x1b');
        assert_eq!(encoded[1], b'[');
        assert_eq!(encoded[2], b'M');
        assert_eq!(encoded[3], 32); // button code + 32
        assert_eq!(encoded[4], 43); // col + 1 + 32
        assert_eq!(encoded[5], 38); // row + 1 + 32
    }

    #[test]
    fn test_mouse_event_utf8_encoding() {
        let event = MouseEvent::new(1, 20, 10, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Utf8);
        assert_eq!(encoded.len(), 6);
        assert_eq!(encoded[0], b'\x1b');
        assert_eq!(encoded[1], b'[');
        assert_eq!(encoded[2], b'M');
        assert_eq!(encoded[3], 33); // button 1 + 32
        assert_eq!(encoded[4], 53); // col + 1 + 32
        assert_eq!(encoded[5], 43); // row + 1 + 32
    }

    #[test]
    fn test_mouse_event_urxvt_encoding() {
        let event = MouseEvent::new(0, 15, 7, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Urxvt);
        let expected = b"\x1b[32;16;8M";
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_mouse_event_urxvt_release() {
        let event = MouseEvent::new(0, 15, 7, false, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Urxvt);
        let expected = b"\x1b[35;16;8M";
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_mouse_event_with_modifiers() {
        // Test with Shift modifier (bit 0 set = 1)
        let event = MouseEvent::new(0, 5, 3, true, 1);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        // Button code = 0 | (1 << 2) = 4
        let expected = b"\x1b[<4;6;4M";
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_mouse_event_with_multiple_modifiers() {
        // Test with Shift+Ctrl modifiers (bits 0 and 1 set = 3)
        let event = MouseEvent::new(0, 5, 3, true, 3);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        // Button code = 0 | (3 << 2) = 12
        let expected = b"\x1b[<12;6;4M";
        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_mouse_event_buttons() {
        // Test different buttons
        let event1 = MouseEvent::new(0, 0, 0, true, 0); // Left button
        let event2 = MouseEvent::new(1, 0, 0, true, 0); // Middle button
        let event3 = MouseEvent::new(2, 0, 0, true, 0); // Right button

        let encoded1 = event1.encode(MouseMode::Normal, MouseEncoding::Sgr);
        let encoded2 = event2.encode(MouseMode::Normal, MouseEncoding::Sgr);
        let encoded3 = event3.encode(MouseMode::Normal, MouseEncoding::Sgr);

        assert_eq!(encoded1, b"\x1b[<0;1;1M");
        assert_eq!(encoded2, b"\x1b[<1;1;1M");
        assert_eq!(encoded3, b"\x1b[<2;1;1M");
    }

    #[test]
    fn test_mouse_event_large_coordinates() {
        // Test with large coordinates (beyond 223 for default encoding)
        let event = MouseEvent::new(0, 250, 200, true, 0);

        // SGR should handle large coordinates
        let sgr = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        assert!(String::from_utf8_lossy(&sgr).contains("251"));
        assert!(String::from_utf8_lossy(&sgr).contains("201"));

        // Default encoding clamps coordinates: (col + 1).min(223) + 32
        // col = 250, (250 + 1).min(223) = 223, 223 + 32 = 255
        // row = 200, (200 + 1).min(223) = 201, 201 + 32 = 233
        let default = event.encode(MouseMode::Normal, MouseEncoding::Default);
        assert_eq!(default[4], 255); // col: (250 + 1).min(223) + 32 = 223 + 32 = 255
        assert_eq!(default[5], 233); // row: (200 + 1).min(223) + 32 = 201 + 32 = 233
    }

    #[test]
    fn test_mouse_event_utf8_large_coordinates_clamped() {
        // Large coordinates should be clamped similarly to default encoding
        let event = MouseEvent::new(0, 250, 200, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Utf8);

        assert_eq!(encoded.len(), 6);
        assert_eq!(encoded[0], b'\x1b');
        assert_eq!(encoded[1], b'[');
        assert_eq!(encoded[2], b'M');
        assert_eq!(encoded[3], 32); // button code + 32
        assert_eq!(encoded[4], 255); // col: (250 + 1).min(223) + 32 = 223 + 32 = 255
        assert_eq!(encoded[5], 233); // row: (200 + 1).min(223) + 32 = 201 + 32 = 233
    }

    #[test]
    fn test_mouse_event_at_origin() {
        let event = MouseEvent::new(0, 0, 0, true, 0);
        let encoded = event.encode(MouseMode::Normal, MouseEncoding::Sgr);
        assert_eq!(encoded, b"\x1b[<0;1;1M");
    }

    #[test]
    fn test_mouse_mode_equality() {
        assert_eq!(MouseMode::Off, MouseMode::Off);
        assert_eq!(MouseMode::Normal, MouseMode::Normal);
        assert_ne!(MouseMode::Off, MouseMode::Normal);
        assert_ne!(MouseMode::Normal, MouseMode::AnyEvent);
    }

    #[test]
    fn test_mouse_encoding_equality() {
        assert_eq!(MouseEncoding::Default, MouseEncoding::Default);
        assert_eq!(MouseEncoding::Sgr, MouseEncoding::Sgr);
        assert_ne!(MouseEncoding::Default, MouseEncoding::Sgr);
        assert_ne!(MouseEncoding::Utf8, MouseEncoding::Urxvt);
    }

    #[test]
    fn test_mouse_event_equality() {
        let event1 = MouseEvent::new(0, 10, 5, true, 0);
        let event2 = MouseEvent::new(0, 10, 5, true, 0);
        let event3 = MouseEvent::new(1, 10, 5, true, 0);

        assert_eq!(event1, event2);
        assert_ne!(event1, event3);
    }
}
