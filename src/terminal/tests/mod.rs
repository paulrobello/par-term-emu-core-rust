//! Terminal implementation tests

#[cfg(test)]
mod attributes;
#[cfg(test)]
mod basic;
#[cfg(test)]
mod bookmarks;
#[cfg(test)]
mod clipboard;
#[cfg(test)]
mod cursor;
#[cfg(test)]
mod detection;
#[cfg(test)]
mod editing;
#[cfg(test)]
mod ffi_tests;
#[cfg(test)]
mod grid_integration_tests;
#[cfg(test)]
mod keyboard;
#[cfg(test)]
mod kitty_apc;
#[cfg(test)]
mod modes;
#[cfg(test)]
mod observer_tests;
#[cfg(test)]
mod queries;
#[cfg(test)]
mod rectangles;
#[cfg(test)]
mod replay_snapshot_tests;
#[cfg(test)]
mod scrolling;
#[cfg(test)]
mod search;
#[cfg(test)]
mod shell_integration;
#[cfg(test)]
mod terminal_tests;
#[cfg(test)]
mod tmux;
#[cfg(test)]
mod tui;
#[cfg(test)]
mod vt520;

#[cfg(test)]
mod metrics;
#[cfg(test)]
mod screen;

#[cfg(test)]
mod arc006_events {
    use crate::terminal::{Terminal, TerminalEvent, MAX_TERMINAL_EVENTS};

    #[test]
    fn terminal_events_capped_with_oldest_eviction() {
        let mut term = Terminal::new(80, 24);
        // Push one more than the cap; tag each with its index for ordering.
        for i in 0..=MAX_TERMINAL_EVENTS {
            term.events
                .terminal_events
                .push(TerminalEvent::ModeChanged(i.to_string(), true));
        }
        assert_eq!(
            term.events.terminal_events.len(),
            MAX_TERMINAL_EVENTS + 1,
            "precondition"
        );

        // Pretend the first half were already dispatched to observers — those
        // are the safe ones to evict first.
        term.events.events_dispatched_up_to = MAX_TERMINAL_EVENTS / 2;

        term.cap_terminal_events();

        assert_eq!(term.events.terminal_events.len(), MAX_TERMINAL_EVENTS);
        // Oldest (i=0) evicted; the new front is the former i=1 event.
        match &term.events.terminal_events[0] {
            TerminalEvent::ModeChanged(name, _) => assert_eq!(name.as_str(), "1"),
            _ => panic!("expected ModeChanged at the front after eviction"),
        }
        // The dispatch index shifts down by exactly the number evicted.
        assert_eq!(
            term.events.events_dispatched_up_to,
            MAX_TERMINAL_EVENTS / 2 - 1
        );
    }

    #[test]
    fn terminal_events_under_cap_are_untouched() {
        let mut term = Terminal::new(80, 24);
        for i in 0..10 {
            term.events
                .terminal_events
                .push(TerminalEvent::ModeChanged(i.to_string(), true));
        }
        term.cap_terminal_events();
        assert_eq!(term.events.terminal_events.len(), 10);
        match &term.events.terminal_events[0] {
            TerminalEvent::ModeChanged(name, _) => assert_eq!(name.as_str(), "0"),
            _ => panic!("expected first event preserved"),
        }
    }
}
