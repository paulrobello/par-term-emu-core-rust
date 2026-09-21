//! Session, window, and pane identifiers.

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};

/// Error returned when an identifier string is not well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIdError(String);

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid identifier: {}", self.0)
    }
}

impl std::error::Error for ParseIdError {}

macro_rules! mux_id {
    ($name:ident, $sigil:literal, $what:literal) => {
        #[doc = concat!("A ", $what, " identifier, rendered as `", $sigil, "N` on the wire.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $sigil, self.0)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let rest = s
                    .strip_prefix($sigil)
                    .ok_or_else(|| ParseIdError(s.to_string()))?;
                if rest.is_empty() {
                    return Err(ParseIdError(s.to_string()));
                }
                rest.parse::<u32>()
                    .map($name)
                    .map_err(|_| ParseIdError(s.to_string()))
            }
        }
    };
}

mux_id!(SessionId, "$", "session");
mux_id!(WindowId, "@", "window");
mux_id!(PaneId, "%", "pane");

/// Hands out monotonically increasing identifiers, counting each kind
/// independently the way tmux does.
#[derive(Debug, Default)]
pub struct IdAllocator {
    session: AtomicU32,
    window: AtomicU32,
    pane: AtomicU32,
}

impl IdAllocator {
    /// Create an allocator whose counters all start at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate the next session identifier.
    pub fn next_session(&self) -> SessionId {
        SessionId(self.session.fetch_add(1, Ordering::Relaxed))
    }

    /// Allocate the next window identifier.
    pub fn next_window(&self) -> WindowId {
        WindowId(self.window.fetch_add(1, Ordering::Relaxed))
    }

    /// Allocate the next pane identifier.
    pub fn next_pane(&self) -> PaneId {
        PaneId(self.pane.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_display_with_tmux_sigils() {
        assert_eq!(SessionId(0).to_string(), "$0");
        assert_eq!(WindowId(1).to_string(), "@1");
        assert_eq!(PaneId(3).to_string(), "%3");
    }

    #[test]
    fn ids_round_trip_through_parse() {
        assert_eq!("$0".parse::<SessionId>().unwrap(), SessionId(0));
        assert_eq!("@1".parse::<WindowId>().unwrap(), WindowId(1));
        assert_eq!("%3".parse::<PaneId>().unwrap(), PaneId(3));
    }

    #[test]
    fn parse_rejects_the_wrong_sigil() {
        assert!("@0".parse::<SessionId>().is_err(), "@ is a window sigil");
        assert!("%0".parse::<WindowId>().is_err(), "% is a pane sigil");
        assert!("$0".parse::<PaneId>().is_err(), "$ is a session sigil");
    }

    #[test]
    fn parse_rejects_malformed_input() {
        assert!("".parse::<PaneId>().is_err());
        assert!("%".parse::<PaneId>().is_err());
        assert!("%abc".parse::<PaneId>().is_err());
        assert!("3".parse::<PaneId>().is_err(), "bare number has no sigil");
    }

    #[test]
    fn allocator_hands_out_increasing_ids() {
        let alloc = IdAllocator::new();
        assert_eq!(alloc.next_pane(), PaneId(0));
        assert_eq!(alloc.next_pane(), PaneId(1));
        // Each kind counts independently, as tmux does.
        assert_eq!(alloc.next_window(), WindowId(0));
        assert_eq!(alloc.next_session(), SessionId(0));
    }
}
