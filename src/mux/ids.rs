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
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(pub u32);

        impl SigilId for $name {
            const SIGIL: &'static str = $sigil;
        }

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

/// A typed id kind whose wire sigil drives [`Target::parse`] — the prefix
/// that marks a value as a typed id rather than a name.
pub trait SigilId: Sized {
    /// The sigil (`%`, `@`, `$`) a target value must start with to be
    /// parsed as this id kind.
    const SIGIL: &'static str;
}

/// A command target that may be a typed id or a name, resolved against the
/// tree daemon-side (the parser has no tree to resolve names against).
///
/// A `-t` value starting with the kind's sigil must parse as a typed id —
/// a malformed one (`%abc`) keeps the parser's invalid-target error, and a
/// pane TITLED `%3` can never shadow pane `%3` because the sigil prefix
/// always means the id. Any other value is a name, matched exactly against
/// the pane's user title, the window's name, or the session's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target<I> {
    /// A typed `%N`/`@N`/`$N` id.
    Id(I),
    /// A name, resolved by the tree: pane user titles, window names,
    /// session names.
    Name(String),
}

impl<I: SigilId + FromStr<Err = ParseIdError>> Target<I> {
    /// Classify a raw `-t` value: sigil-prefixed values parse as ids (a
    /// malformed one errors, as before name targets existed), any other
    /// value becomes a name.
    pub fn parse(raw: &str) -> Result<Self, ParseIdError> {
        if raw.starts_with(I::SIGIL) {
            raw.parse::<I>().map(Target::Id)
        } else {
            Ok(Target::Name(raw.to_string()))
        }
    }
}

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

    /// The next identifier each kind will hand out — `(session, window,
    /// pane)`, in that order.
    ///
    /// The persistence path stores these counters (par-mux.md D3.5) so a
    /// restored server's new panes do not collide with restored ones.
    pub fn next_ids(&self) -> (u32, u32, u32) {
        (
            self.session.load(Ordering::Relaxed),
            self.window.load(Ordering::Relaxed),
            self.pane.load(Ordering::Relaxed),
        )
    }

    /// Resume an allocator whose counters were captured by
    /// [`Self::next_ids`] — the restore-side counterpart.
    pub fn resume(next: (u32, u32, u32)) -> Self {
        let (session, window, pane) = next;
        Self {
            session: AtomicU32::new(session),
            window: AtomicU32::new(window),
            pane: AtomicU32::new(pane),
        }
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

    #[test]
    fn target_parse_classifies_sigil_vs_name() {
        assert_eq!(
            Target::<PaneId>::parse("%3").unwrap(),
            Target::Id(PaneId(3))
        );
        assert_eq!(
            Target::<PaneId>::parse("build").unwrap(),
            Target::Name("build".to_string())
        );
        assert_eq!(
            Target::<SessionId>::parse("work").unwrap(),
            Target::Name("work".to_string())
        );
        assert_eq!(
            Target::<WindowId>::parse("logs").unwrap(),
            Target::Name("logs".to_string())
        );
    }

    #[test]
    fn target_parse_treats_a_foreign_sigil_as_a_name() {
        // Only the target KIND's own sigil means "typed id" — a foreign
        // sigil is just characters, so "@0" as a pane target is a name
        // that resolves only against a pane literally titled "@0" (and
        // otherwise errors at resolution as an unknown pane).
        assert_eq!(
            Target::<PaneId>::parse("@0").unwrap(),
            Target::Name("@0".to_string())
        );
        assert_eq!(
            Target::<SessionId>::parse("%1").unwrap(),
            Target::Name("%1".to_string())
        );
    }

    #[test]
    fn target_parse_sigil_prefix_always_means_the_id() {
        // Ids win over names: "%3" classifies as the id even when some
        // pane's title happens to be "%3" (the tree-side rule the
        // ambiguity tests cover).
        assert_eq!(
            Target::<PaneId>::parse("%3").unwrap(),
            Target::Id(PaneId(3)),
            "a sigil-prefixed value never becomes a name"
        );
        // Malformed sigil values keep the invalid-target error shape.
        assert!(Target::<PaneId>::parse("%abc").is_err());
        assert!(Target::<PaneId>::parse("%").is_err());
    }
}
