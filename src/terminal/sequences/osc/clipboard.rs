//! Clipboard OSC sequence handling

use crate::terminal::Terminal;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

impl Terminal {
    pub(crate) fn handle_osc_clipboard(&mut self, _command: &str, params: &[&[u8]]) {
        // Format: OSC 52 ; selection ; data ST
        if params.len() >= 3 {
            if let Ok(selection) = std::str::from_utf8(params[1]) {
                if let Ok(data) = std::str::from_utf8(params[2]) {
                    let data = data.trim();

                    if selection.contains('c') || selection.is_empty() {
                        if data == "?" {
                            if self.allow_clipboard_read {
                                if let Some(content) = &self.clipboard_content {
                                    let encoded = BASE64.encode(content.as_bytes());
                                    let response = format!("\x1b]52;c;{}\x1b\\", encoded);
                                    self.push_response(response.as_bytes());
                                } else {
                                    let response = b"\x1b]52;c;\x1b\\";
                                    self.push_response(response);
                                }
                            }
                        } else if !data.is_empty() {
                            if let Ok(decoded_bytes) = BASE64.decode(data.as_bytes()) {
                                if let Ok(text) = String::from_utf8(decoded_bytes) {
                                    self.clipboard_content = Some(text);
                                }
                            }
                        } else {
                            self.clipboard_content = None;
                        }
                    }
                }
            }
        }
    }
}
