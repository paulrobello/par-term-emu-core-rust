//! Host-input encoding (ARC-003): mouse and focus reports and bracketed paste.

use super::Terminal;
use crate::mouse::{MouseEvent, MouseMode};

impl Terminal {
    /// Report mouse event
    pub fn report_mouse(&mut self, event: MouseEvent) -> Vec<u8> {
        if self.modes.mouse_mode == MouseMode::Off {
            return Vec::new();
        }
        if self.modes.mouse_mode == MouseMode::X10 {
            // X10 reports button presses only — no release, no motion
            // (button 3 = no button), and no modifier bits
            if !event.pressed || event.button == 3 {
                return Vec::new();
            }
            let mut press = event;
            press.modifiers = 0;
            return press.encode(self.modes.mouse_mode, self.modes.mouse_encoding);
        }
        event.encode(self.modes.mouse_mode, self.modes.mouse_encoding)
    }

    /// Report focus in event
    pub fn report_focus_in(&self) -> Vec<u8> {
        if self.modes.focus_tracking {
            b"\x1b[I".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Report focus out event
    pub fn report_focus_out(&self) -> Vec<u8> {
        if self.modes.focus_tracking {
            b"\x1b[O".to_vec()
        } else {
            Vec::new()
        }
    }

    /// Get bracketed paste start sequence
    pub fn bracketed_paste_start(&self) -> &[u8] {
        if self.modes.bracketed_paste {
            b"\x1b[200~"
        } else {
            b""
        }
    }

    /// Get bracketed paste end sequence
    pub fn bracketed_paste_end(&self) -> &[u8] {
        if self.modes.bracketed_paste {
            b"\x1b[201~"
        } else {
            b""
        }
    }

    /// Process pasted content with proper bracketing if enabled
    ///
    /// If bracketed paste mode is enabled, wraps the content with ESC[200~ and ESC[201~
    /// Otherwise, processes the content directly
    pub fn paste(&mut self, content: &str) {
        if self.modes.bracketed_paste {
            // Send: ESC[200~ + content + ESC[201~
            self.process(b"\x1b[200~");
            self.process(content.as_bytes());
            self.process(b"\x1b[201~");
        } else {
            // Send content directly
            self.process(content.as_bytes());
        }
    }
}
