//! Title-related OSC sequence handling
//!
//! Capability boundary (ARC-002): the handler takes only the title state and
//! the event broker, not `&mut Terminal`.

use crate::terminal::{EventBroker, TerminalEvent, TitleState};

/// OSC 0/2 (set title), OSC 21 (push title), OSC 22/23 (pop title).
pub(crate) fn handle_osc_title(
    title_state: &mut TitleState,
    events: &mut EventBroker,
    command: &str,
    params: &[&[u8]],
) {
    match command {
        "0" | "2" if params.len() >= 2 => {
            if let Ok(title) = std::str::from_utf8(params[1]) {
                let new_title = title.to_string();
                if title_state.title != new_title {
                    title_state.title = new_title.clone();
                    events.push(TerminalEvent::TitleChanged(new_title));
                }
            }
        }
        "21" => {
            if params.len() >= 2 {
                if let Ok(title) = std::str::from_utf8(params[1]) {
                    title_state.title_stack.push(title.to_string());
                }
            } else {
                title_state.title_stack.push(title_state.title.clone());
            }
        }
        "22" | "23" => {
            if let Some(title) = title_state.title_stack.pop() {
                title_state.title = title;
            }
        }
        _ => {}
    }
}
