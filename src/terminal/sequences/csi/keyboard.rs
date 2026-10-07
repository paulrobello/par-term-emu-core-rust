//! Kitty keyboard protocol CSI sequence handling
//!
//! Capability boundary (ARC-002): the handler takes only the keyboard
//! protocol state and the reply buffer, not `&mut Terminal`.

use crate::terminal::KeyboardState;
use vte::Params;

/// Kitty keyboard protocol (`CSI = / ? / > / < … u`).
pub(crate) fn handle_csi_keyboard(
    keyboard: &mut KeyboardState,
    response: &mut Vec<u8>,
    action: char,
    params: &Params,
    intermediates: &[u8],
) {
    if action == 'u' {
        // Kitty keyboard protocol
        if intermediates.contains(&b'?') {
            // Query current flags: CSI ? u
            let reply = format!("\x1b[?{}u", keyboard.keyboard_flags);
            response.extend_from_slice(reply.as_bytes());
        } else if intermediates.contains(&b'>') {
            // Push flags: CSI > flags u
            let mut iter = params.iter();
            if let Some(param_slice) = iter.next() {
                let flags = param_slice.first().copied().unwrap_or(0);
                keyboard.keyboard_stack.push(keyboard.keyboard_flags);
                keyboard.keyboard_flags = flags;
            }
        } else if intermediates.contains(&b'<') {
            // Pop flags: CSI < n u
            let mut iter = params.iter();
            let n = iter.next().and_then(|p| p.first()).copied().unwrap_or(1) as usize;
            for _ in 0..n {
                if let Some(flags) = keyboard.keyboard_stack.pop() {
                    keyboard.keyboard_flags = flags;
                }
            }
        } else {
            // Set/Unset flags: CSI [=] flags ; mode u
            let mut iter = params.iter();
            if let Some(param_slice) = iter.next() {
                let flags = param_slice.first().copied().unwrap_or(0);
                let mode = iter.next().and_then(|p| p.first()).copied().unwrap_or(1);

                match mode {
                    1 => keyboard.keyboard_flags = flags,  // Set
                    2 => keyboard.keyboard_flags |= flags, // Add
                    3 => {
                        // Report (as per par-term tests)
                        let reply = format!("\x1b[?{}u", keyboard.keyboard_flags);
                        response.extend_from_slice(reply.as_bytes());
                    }
                    _ => keyboard.keyboard_flags = flags, // Default to set
                }
            }
        }
    }
}
