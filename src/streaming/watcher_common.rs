//! The reconnect policy shared by the streaming daemon watchers.
//!
//! [`super::RosterWatcher`] and [`super::TreeWatcher`] redial the daemon
//! control socket through one backoff policy: a failed or short-lived
//! connect/fetch/serve cycle doubles the delay toward [`BACKOFF_MAX`], and
//! only a connection that held [`BACKOFF_STABLE`] or longer resets it to
//! [`BACKOFF_INITIAL`].

use std::time::Duration;

/// The first redial delay, and what a stable connection resets it to.
pub(super) const BACKOFF_INITIAL: Duration = Duration::from_millis(100);
/// The cap the cumulative redial delay grows toward.
pub(super) const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// A connection must hold at least this long for its end to reset the
/// redial backoff to [`BACKOFF_INITIAL`]. Anything shorter is a flap: the
/// flap keeps the cumulative (capped) delay instead of redialing at 100ms
/// forever.
pub(super) const BACKOFF_STABLE: Duration = Duration::from_secs(30);

/// The redial delay after one connect/fetch/serve cycle ends: a failed or
/// short-lived cycle grows the cumulative backoff toward [`BACKOFF_MAX`],
/// and only a connection that held [`BACKOFF_STABLE`] or longer resets it
/// to [`BACKOFF_INITIAL`]. Passing the grown value forward is what keeps a
/// connect-then-drop flap from redialing at the initial 100ms forever.
pub(super) fn next_backoff(current: Duration, held: Option<Duration>) -> Duration {
    match held {
        Some(held) if held >= BACKOFF_STABLE => BACKOFF_INITIAL,
        _ => (current * 2).min(BACKOFF_MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short-lived connection is a flap: the delay grows past the
    /// initial 100ms instead of resetting to it, capping at BACKOFF_MAX.
    #[test]
    fn flap_grows_the_backoff_and_stability_resets_it() {
        assert_eq!(
            next_backoff(BACKOFF_INITIAL, None),
            Duration::from_millis(200)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(4), None),
            BACKOFF_MAX,
            "growth caps at BACKOFF_MAX"
        );
        assert_eq!(next_backoff(BACKOFF_MAX, None), BACKOFF_MAX);
        // A connection that held past BACKOFF_STABLE proves the daemon
        // healthy, and the next redial starts over at the initial delay.
        assert_eq!(
            next_backoff(BACKOFF_MAX, Some(BACKOFF_STABLE)),
            BACKOFF_INITIAL
        );
        // One millisecond under the stability mark still counts as a flap.
        assert_eq!(
            next_backoff(
                BACKOFF_INITIAL,
                Some(BACKOFF_STABLE - Duration::from_millis(1))
            ),
            Duration::from_millis(200)
        );
    }
}
