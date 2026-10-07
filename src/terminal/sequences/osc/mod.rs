//! OSC (Operating System Command) sequence handling dispatcher

mod clipboard;
mod color;
mod iterm;
mod notify;
mod shell;
mod title;

use crate::debug;
use crate::terminal::{EventBroker, HyperlinkState, Terminal};
use std::num::NonZeroU32;

impl Terminal {
    /// Check if an OSC command should be filtered due to security settings
    pub(crate) fn is_insecure_osc(&self, command: &str) -> bool {
        if !self.security_state.disable_insecure_sequences {
            return false;
        }

        matches!(command, "52" | "8" | "9" | "777" | "99")
    }

    /// VTE OSC dispatch - handle OSC sequences
    pub(in crate::terminal) fn osc_dispatch_impl(
        &mut self,
        params: &[&[u8]],
        _bell_terminated: bool,
    ) {
        debug::log_osc_dispatch(params);
        // SEC-003: the incremental guard in `advance_parser` already dropped
        // the over-cap payload bytes; drop the truncated dispatch whole.
        if self.security_state.osc_discard_dispatch {
            self.security_state.osc_discard_dispatch = false;
            return;
        }
        if params.is_empty() {
            return;
        }

        // Reject excessively large OSC data to prevent memory exhaustion (QA-012).
        let max = self.security_state.max_osc_data_length;
        let total_len: usize = params.iter().map(|p| p.len()).sum();
        if total_len > max {
            crate::debug_log!(
                "OSC",
                "OSC data too large: {} bytes (max {}), ignoring",
                total_len,
                max
            );
            return;
        }

        if let Ok(command) = std::str::from_utf8(params[0]) {
            if self.is_insecure_osc(command) {
                crate::debug_log!(
                    "SECURITY",
                    "Blocked insecure OSC {} (disable_insecure_sequences=true)",
                    command
                );
                return;
            }

            match command {
                "0" | "2" | "21" | "22" | "23" => title::handle_osc_title(
                    &mut self.title_state,
                    &mut self.events,
                    command,
                    params,
                ),
                "7" | "133" => self.handle_osc_shell(command, params),
                "8" => handle_osc_hyperlink(
                    &mut self.hyperlink_state,
                    &mut self.events,
                    (self.cursor.row, self.cursor.col),
                    params,
                ),
                "9" | "777" | "934" | "99" => notify::handle_osc_notify(
                    &mut self.notifications_state,
                    &mut self.progress_state,
                    &mut self.events,
                    command,
                    params,
                ),
                "52" => clipboard::handle_osc_clipboard(
                    &mut self.clipboard_state,
                    &mut self.response_buffer,
                    params,
                ),
                "4" | "104" | "10" | "11" | "12" | "110" | "111" | "112" => {
                    color::handle_osc_color(
                        &mut self.theme,
                        &mut self.response_buffer,
                        self.security_state.disable_insecure_sequences,
                        command,
                        params,
                    )
                }
                "1337" => self.handle_osc_iterm(command, params),
                _ => {
                    crate::debug_log!("OSC", "Unsupported OSC command: {}", command);
                }
            }
        }
    }
}

/// OSC 8 hyperlink open/close.
///
/// Capability boundary (ARC-002): takes the hyperlink table, the event
/// broker, and the cursor position (row, col) rather than `&mut Terminal`.
pub(crate) fn handle_osc_hyperlink(
    hyperlink_state: &mut HyperlinkState,
    events: &mut EventBroker,
    (row, col): (usize, usize),
    params: &[&[u8]],
) {
    if params.len() >= 3 {
        if let Ok(url) = std::str::from_utf8(params[2]) {
            let url = url.trim();

            if url.is_empty() {
                hyperlink_state.current_hyperlink_id = None;
            } else {
                let id = hyperlink_state
                    .hyperlinks
                    .iter()
                    .find(|(_, v)| v.as_str() == url)
                    .map(|(k, _)| *k)
                    .unwrap_or_else(|| {
                        let id = hyperlink_state.next_hyperlink_id;
                        hyperlink_state.hyperlinks.insert(id, url.to_string());
                        hyperlink_state.next_hyperlink_id += 1;
                        id
                    });

                // id >= 1 (next_hyperlink_id starts at 1); store as the
                // niche-optimized NonZeroU32 used on cells (ARC-010).
                hyperlink_state.current_hyperlink_id = NonZeroU32::new(id);

                events.push(crate::terminal::TerminalEvent::HyperlinkAdded {
                    url: url.to_string(),
                    row,
                    col,
                    id: Some(id),
                });
            }
        }
    } else if params.len() == 2 {
        hyperlink_state.current_hyperlink_id = None;
    }
}

#[cfg(test)]
mod tests;
