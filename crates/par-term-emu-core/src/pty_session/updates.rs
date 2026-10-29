//! The update generation counter and waiters on it.

use super::*;

impl PtySession {
    /// Get the current update generation number
    ///
    /// This number is incremented every time the terminal content changes.
    /// Useful for detecting when to redraw in event loops.
    ///
    /// # Returns
    /// The current generation number
    pub fn update_generation(&self) -> u64 {
        self.update_generation.load(Ordering::SeqCst)
    }

    /// Epoch ms of the last processed output read, or 0 when none has been
    /// processed yet. Stamped by the PTY reader thread on every read and by
    /// [`PtySession::mark_updated`] for process-fed sessions — the liveness
    /// signal the par-mux roster's `last_activity_at` reads.
    pub fn last_output_epoch_ms(&self) -> u64 {
        self.last_output_at.load(Ordering::Relaxed)
    }

    /// Mark the session's content as updated from OUTSIDE the PTY reader
    /// thread — the generation bump + waiter wake the reader performs on
    /// every read. For a session with no child (par-mux panes fed via
    /// `process_data`), nothing else ever advances the generation, so a
    /// generation-keyed render cache would serve stale cells forever.
    pub fn mark_updated(&self) {
        self.update_generation.fetch_add(1, Ordering::SeqCst);
        self.last_output_at.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            Ordering::Relaxed,
        );
        let _guard = self.update_signal.0.lock();
        self.update_signal.1.notify_all();
    }

    /// Check if the terminal has been updated since a given generation
    ///
    /// # Arguments
    /// * `last_generation` - The generation number from a previous call to `update_generation()`
    ///
    /// # Returns
    /// True if updates have occurred since the given generation
    pub fn has_updates_since(&self, last_generation: u64) -> bool {
        self.update_generation() > last_generation
    }

    /// A shareable handle to the session's wait primitives (ENH-011).
    ///
    /// `PtySession` itself is not `Sync` (its master PTY handle is
    /// `Send`-only), so a Python binding that releases the GIL cannot hold
    /// `&PtySession` across the wait. This handle carries only the `Arc`s
    /// the wait needs and is `Send + Sync + Clone`.
    pub fn update_waiter(&self) -> UpdateWaiter {
        UpdateWaiter {
            terminal: Arc::clone(&self.terminal),
            signal: Arc::clone(&self.update_signal),
            generation: Arc::clone(&self.update_generation),
            running: Arc::clone(&self.running),
        }
    }

    /// Block until the update generation advances past `since` or `timeout`
    /// elapses — see [`UpdateWaiter::wait_for_update`].
    pub fn wait_for_update(&self, since: u64, timeout: std::time::Duration) -> Option<u64> {
        self.update_waiter().wait_for_update(since, timeout)
    }

    /// Block until `predicate` holds on the terminal, re-checking after
    /// every applied update, or until `timeout` elapses — see
    /// [`UpdateWaiter::wait_until`].
    pub fn wait_until(
        &self,
        timeout: std::time::Duration,
        predicate: impl Fn(&Terminal) -> bool,
    ) -> bool {
        self.update_waiter().wait_until(timeout, predicate)
    }
}
