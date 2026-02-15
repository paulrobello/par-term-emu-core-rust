//! Kitty keyboard protocol CSI sequence handling

use crate::terminal::Terminal;
use vte::Params;

impl Terminal {
    pub(crate) fn handle_csi_keyboard(
        &mut self,
        action: char,
        params: &Params,
        intermediates: &[u8],
    ) {
        if action == 'u' {
            // Kitty keyboard protocol
            if intermediates.contains(&b'?') {
                // Query current flags: CSI ? u
                let response = format!("\x1b[?{}u", self.keyboard_flags);
                self.push_response(response.as_bytes());
            } else if intermediates.contains(&b'>') {
                // Push flags: CSI > flags u
                let mut iter = params.iter();
                if let Some(param_slice) = iter.next() {
                    let flags = param_slice.first().copied().unwrap_or(0);
                    self.keyboard_stack.push(self.keyboard_flags);
                    self.keyboard_flags = flags;
                }
            } else if intermediates.contains(&b'<') {
                // Pop flags: CSI < n u
                let mut iter = params.iter();
                let n = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                for _ in 0..n {
                    if let Some(flags) = self.keyboard_stack.pop() {
                        self.keyboard_flags = flags;
                    }
                }
            } else {
                // Set/Unset flags: CSI [=] flags ; mode u
                let mut iter = params.iter();
                if let Some(param_slice) = iter.next() {
                    let flags = param_slice.first().copied().unwrap_or(0);
                    let mode = iter.next().and_then(|p| p.first()).copied().unwrap_or(1);

                    match mode {
                        1 => self.keyboard_flags = flags,  // Set
                        2 => self.keyboard_flags |= flags, // Add
                        3 => {
                            // Report (as per par-term tests)
                            let response = format!("\x1b[?{}u", self.keyboard_flags);
                            self.push_response(response.as_bytes());
                        }
                        _ => self.keyboard_flags = flags, // Default to set
                    }
                }
            }
        }
    }
}
