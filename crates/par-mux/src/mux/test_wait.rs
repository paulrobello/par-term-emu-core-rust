//! Load-independent test waits, shared by the lib's unit tests and the
//! `tests/` integration suites (which include this file by `#[path]`).
//! Std-only and test-only: it is never part of the published API.
//!
//! A **positive wait** ("until X happens") fails only when X never
//! happens. Its budget is a hang detector, not a latency assertion, so it
//! is the fixed, generous [`CEILING`] polled fast: a healthy run returns
//! as soon as X holds, a starved host only delays it, and a wedged daemon
//! still fails at the ceiling. Budgets calibrated to an idle machine
//! (5-15 s) failed at host load 200-560 before X could occur.
//!
//! A **negative window** ("nothing happens for W") keeps its own duration:
//! the ceiling would make every passing run pay it. Where a window stood
//! in for a condition (a quiet screen meaning "the repaint landed"), the
//! tests now wait for the condition itself.

#![allow(dead_code)]

use std::time::{Duration, Instant};

/// The hang ceiling for every positive wait. Below the nextest ci
/// profile's per-test kill (`.config/nextest.toml`), so a wedged wait
/// fails with its own message first.
pub const CEILING: Duration = Duration::from_secs(90);

/// How often a positive wait re-checks its condition.
pub const POLL: Duration = Duration::from_millis(25);

/// Poll `check` every [`POLL`] until it yields `Some`, within `ceiling`.
/// On timeout, panics naming `what` with the last observed state from
/// `last` (the condition's own diagnostic, e.g. the last reply).
pub fn until<T>(
    what: &str,
    ceiling: Duration,
    mut check: impl FnMut() -> Option<T>,
    last: impl Fn() -> String,
) -> T {
    let end = Instant::now() + ceiling;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(
            Instant::now() < end,
            "{what}: not satisfied within {ceiling:?}; last state: {}",
            last()
        );
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_returns_the_first_satisfied_value() {
        let mut n = 0;
        let got = until(
            "count to 3",
            CEILING,
            || {
                n += 1;
                (n >= 3).then_some(n)
            },
            String::new,
        );
        assert_eq!(got, 3);
    }

    /// The hang detector: a condition that never holds fails at the
    /// ceiling it was given, naming what it waited for.
    #[test]
    fn until_fails_at_its_ceiling_with_the_last_state() {
        let started = Instant::now();
        let caught = std::panic::catch_unwind(|| {
            until(
                "a reply that never comes",
                Duration::from_millis(200),
                || None::<()>,
                || "still waiting".to_string(),
            )
        });
        let elapsed = started.elapsed();
        let message = caught
            .expect_err("a never-satisfied wait fails")
            .downcast::<String>()
            .map(|s| *s)
            .unwrap_or_default();
        assert!(message.contains("a reply that never comes"), "{message}");
        assert!(message.contains("still waiting"), "{message}");
        assert!(
            elapsed >= Duration::from_millis(200) && elapsed < Duration::from_secs(10),
            "failed at its ceiling: {elapsed:?}"
        );
    }
}
