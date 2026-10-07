//! Clipboard OSC sequence handling
//!
//! Capability boundary (ARC-002): the handler takes only the OSC 52
//! clipboard state and the reply buffer, not `&mut Terminal`.

use crate::terminal::ClipboardState;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

/// OSC 52 clipboard write / query.
pub(crate) fn handle_osc_clipboard(
    clipboard: &mut ClipboardState,
    response: &mut Vec<u8>,
    params: &[&[u8]],
) {
    // Format: OSC 52 ; selection ; data ST
    if params.len() >= 3 {
        if let Ok(selection) = std::str::from_utf8(params[1]) {
            if let Ok(data) = std::str::from_utf8(params[2]) {
                let data = data.trim();

                if selection.contains('c') || selection.is_empty() {
                    if data == "?" {
                        if clipboard.allow_clipboard_read {
                            if let Some(content) = &clipboard.clipboard_content {
                                let encoded = BASE64.encode(content.as_bytes());
                                let reply = format!("\x1b]52;c;{}\x1b\\", encoded);
                                response.extend_from_slice(reply.as_bytes());
                            } else {
                                response.extend_from_slice(b"\x1b]52;c;\x1b\\");
                            }
                        }
                    } else if !data.is_empty() {
                        if let Ok(decoded_bytes) = BASE64.decode(data.as_bytes()) {
                            if let Ok(text) = String::from_utf8(decoded_bytes) {
                                clipboard.clipboard_content = Some(text);
                            }
                        }
                    } else {
                        // Treat empty OSC 52 payloads as a no-op instead of clearing.
                        // Some apps/tmux mouse interactions can emit empty clipboard
                        // writes on plain clicks, which would otherwise destroy existing
                        // clipboard contents (including image clipboards in the host app).
                    }
                }
            }
        }
    }
}
