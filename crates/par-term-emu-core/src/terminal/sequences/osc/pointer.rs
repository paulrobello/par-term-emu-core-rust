//! OSC 22 pointer-shape handling (the kitty/ghostty/iTerm2 meaning —
//! the modern ecosystem's reading of OSC 22; xterm's icon-label stack
//! pop stays on OSC 23 and the push on OSC 21).
//!
//! Capability boundary (ARC-002): the handler takes only the pointer
//! state and the event broker, not `&mut Terminal`.

use crate::terminal::{EventBroker, PointerShapeState, TerminalEvent};

/// The requested-shape cap (SEC-105 family): a host-requested name
/// persists in terminal state, so an unbounded one pins memory. 64
/// bytes clears every real cursor name with room to spare.
const MAX_SHAPE_LEN: usize = 64;

/// `OSC 22 ; [shape] ST`: adopt `shape` (`ew-resize`, `text`, …), or
/// reset to the terminal's own pointer when the shape is empty or
/// absent. Oversized names are ignored whole — a truncated name would
/// be a shape the terminal has never heard of.
pub(crate) fn handle_osc_pointer(
    state: &mut PointerShapeState,
    events: &mut EventBroker,
    params: &[&[u8]],
) {
    let requested = match params.get(1) {
        None => None,
        Some([]) => None,
        Some(raw) => match std::str::from_utf8(raw) {
            Ok(shape) if shape.len() <= MAX_SHAPE_LEN => Some(shape.to_string()),
            _ => return,
        },
    };
    if state.shape != requested {
        state.shape = requested.clone();
        events.push(TerminalEvent::PointerShapeChanged(requested));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalEventKind;

    fn broker() -> EventBroker {
        EventBroker::default()
    }

    #[test]
    fn a_shape_is_stored_and_announced() {
        let mut state = PointerShapeState::default();
        let mut events = broker();
        let raw: &[&[u8]] = &[b"22".as_slice(), b"ew-resize".as_slice()];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(state.shape.as_deref(), Some("ew-resize"));
        assert_eq!(events.pending_len(), 1);
        assert_eq!(
            events.terminal_events[0].kind(),
            TerminalEventKind::PointerShapeChanged
        );
    }

    #[test]
    fn an_empty_shape_resets() {
        let mut state = PointerShapeState {
            shape: Some("ew-resize".to_string()),
        };
        let mut events = broker();
        let raw: &[&[u8]] = &[b"22".as_slice(), b"".as_slice()];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(state.shape, None);
        let raw: &[&[u8]] = &[b"22".as_slice()];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(state.shape, None, "absent shape is also a reset");
    }

    #[test]
    fn an_oversized_shape_is_ignored_whole() {
        let mut state = PointerShapeState {
            shape: Some("text".to_string()),
        };
        let mut events = broker();
        let oversized = vec![b'x'; MAX_SHAPE_LEN + 1];
        let raw: &[&[u8]] = &[b"22".as_slice(), &oversized];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(state.shape.as_deref(), Some("text"), "unchanged");
        assert_eq!(
            events.pending_len(),
            0,
            "an ignored request writes no event"
        );
    }

    #[test]
    fn an_invalid_utf8_shape_is_ignored() {
        let mut state = PointerShapeState::default();
        let mut events = broker();
        let raw: &[&[u8]] = &[b"22".as_slice(), [0xff, 0xfe].as_slice()];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(state.shape, None);
        assert_eq!(events.pending_len(), 0);
    }

    #[test]
    fn an_unchanged_shape_writes_no_event() {
        let mut state = PointerShapeState {
            shape: Some("ew-resize".to_string()),
        };
        let mut events = broker();
        let raw: &[&[u8]] = &[b"22".as_slice(), b"ew-resize".as_slice()];
        handle_osc_pointer(&mut state, &mut events, raw);
        assert_eq!(events.pending_len(), 0);
    }
}
