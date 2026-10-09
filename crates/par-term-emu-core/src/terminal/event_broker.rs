//! Event broker: the terminal event queue, bell queue, observer registry,
//! and observer dispatch bookkeeping (ARC-002).
//!
//! `Terminal` owns one [`EventBroker`]. Escape-sequence handlers publish
//! through [`EventBroker::push`] instead of mutating the queue directly, and
//! observer registration, dispatch-batch extraction, and queue capping live
//! here rather than in the `Terminal` impl block. Handlers that only need to
//! publish events can take `&mut EventBroker` as a capability instead of
//! `&mut Terminal`.

use crate::observer::{ObserverEntry, ObserverId, TerminalObserver};
use crate::terminal::{BellEvent, TerminalEvent, TerminalEventKind};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

/// Maximum number of unpolled terminal events retained (ARC-006). Past this,
/// the oldest events are evicted to bound memory under sustained output when
/// the host polls infrequently. Events already dispatched to observers are
/// evicted first; only under extreme load are not-yet-dispatched events dropped.
/// cap: Unpolled terminal events retained from processed output.
pub(crate) const MAX_TERMINAL_EVENTS: usize = 10_000;

/// Maximum number of undrained bell events retained (SEC-207). Past this the
/// oldest bell is dropped, so a BEL flood into a terminal whose host never
/// drains bells cannot grow memory without bound. `Terminal::bell_count`
/// still counts every bell.
/// cap: Undrained bell events retained from processed output.
pub(crate) const MAX_BELL_EVENTS: usize = 1_024;

/// Terminal event queue, bell queue, observer registry, dispatch index, and
/// zone/observer ID counters.
pub(crate) struct EventBroker {
    /// Bell events buffer, capped at [`MAX_BELL_EVENTS`]
    bell_events: VecDeque<BellEvent>,
    /// Terminal events buffer
    pub(super) terminal_events: Vec<TerminalEvent>,
    /// Index of the next event to dispatch to observers (prevents duplicate dispatch)
    pub(super) events_dispatched_up_to: usize,
    /// Registered observers for push-based event delivery
    observers: Vec<ObserverEntry>,
    /// Next observer ID to assign (monotonically increasing)
    next_observer_id: ObserverId,
    /// Next zone ID to assign (monotonically increasing)
    next_zone_id: usize,
    /// Optional event subscription filter for `poll_subscribed_events`
    subscription: Option<HashSet<TerminalEventKind>>,
}

impl Default for EventBroker {
    fn default() -> Self {
        Self {
            bell_events: VecDeque::new(),
            terminal_events: Vec::new(),
            events_dispatched_up_to: 0,
            observers: Vec::new(),
            next_observer_id: 1,
            next_zone_id: 0,
            subscription: None,
        }
    }
}

impl EventBroker {
    /// Queue a terminal event for polling and observer dispatch.
    #[inline]
    pub(crate) fn push(&mut self, event: TerminalEvent) {
        self.terminal_events.push(event);
    }

    /// Queue a bell event (drained separately via [`EventBroker::drain_bells`]).
    /// At [`MAX_BELL_EVENTS`] the oldest pending bell is dropped first.
    #[inline]
    pub(crate) fn push_bell(&mut self, event: BellEvent) {
        if self.bell_events.len() >= MAX_BELL_EVENTS {
            self.bell_events.pop_front();
        }
        self.bell_events.push_back(event);
    }

    /// Take all pending bell events, oldest first.
    pub(crate) fn drain_bells(&mut self) -> Vec<BellEvent> {
        self.bell_events.drain(..).collect()
    }

    /// Allocate the next semantic-zone ID.
    pub(crate) fn alloc_zone_id(&mut self) -> usize {
        let id = self.next_zone_id;
        self.next_zone_id += 1;
        id
    }

    /// Number of pending (unpolled) terminal events.
    pub(crate) fn pending_len(&self) -> usize {
        self.terminal_events.len()
    }

    /// Number of pending (undrained) bell events.
    pub(crate) fn pending_bell_len(&self) -> usize {
        self.bell_events.len()
    }

    /// Register an observer and return its ID.
    pub(crate) fn add_observer(&mut self, observer: Arc<dyn TerminalObserver>) -> ObserverId {
        let id = self.next_observer_id;
        self.next_observer_id += 1;
        self.observers.push(ObserverEntry { id, observer });
        id
    }

    /// Remove an observer by ID; returns whether one was removed.
    pub(crate) fn remove_observer(&mut self, id: ObserverId) -> bool {
        if let Some(pos) = self.observers.iter().position(|o| o.id == id) {
            self.observers.remove(pos);
            true
        } else {
            false
        }
    }

    /// Number of registered observers.
    pub(crate) fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Move the observer registry, observer ID sequence, and subscription
    /// filter from `old` into `self`. Queued events, bells, and the zone ID
    /// counter are not carried (RIS semantics: buffered events reset with
    /// the state they describe).
    pub(crate) fn carry_observers_from(&mut self, old: &mut EventBroker) {
        std::mem::swap(&mut self.observers, &mut old.observers);
        self.next_observer_id = old.next_observer_id;
        std::mem::swap(&mut self.subscription, &mut old.subscription);
    }

    /// Set (`Some`) or clear (`None`) the subscription filter used by
    /// `Terminal::poll_subscribed_events`.
    pub(crate) fn set_subscription(&mut self, filter: Option<HashSet<TerminalEventKind>>) {
        self.subscription = filter;
    }

    /// The current subscription filter (`None` = subscribed to everything).
    #[cfg(test)]
    pub(crate) fn subscription(&self) -> Option<&HashSet<TerminalEventKind>> {
        self.subscription.as_ref()
    }

    /// Remove and return pending events matching the subscription filter;
    /// `None` when no filter is set (the caller then drains everything).
    pub(crate) fn extract_subscribed(&mut self) -> Option<Vec<TerminalEvent>> {
        let filter = self.subscription.take()?;
        let matched = self.extract_matching(|e| filter.contains(&e.kind()));
        self.subscription = Some(filter);
        Some(matched)
    }

    /// Take every pending event and reset the dispatch index.
    pub(crate) fn take_all(&mut self) -> Vec<TerminalEvent> {
        self.events_dispatched_up_to = 0;
        std::mem::take(&mut self.terminal_events)
    }

    /// Drain pending events, splitting each into either an extracted value
    /// (when `try_extract` returns `Ok`) or a leftover event (`Err`) that
    /// stays queued. Powers the typed `poll_*` methods (ARC-006).
    ///
    /// `events_dispatched_up_to` drops by one for every removed event that
    /// sat below it, so it keeps pointing at the first undispatched event of
    /// the compacted queue.
    pub(crate) fn extract<T>(
        &mut self,
        mut try_extract: impl FnMut(TerminalEvent) -> Result<T, TerminalEvent>,
    ) -> Vec<T> {
        let events = std::mem::take(&mut self.terminal_events);
        let dispatched = self.events_dispatched_up_to;
        let mut removed_dispatched = 0;
        let mut extracted = Vec::new();
        let mut remaining = Vec::with_capacity(events.len());
        for (index, event) in events.into_iter().enumerate() {
            match try_extract(event) {
                Ok(value) => {
                    if index < dispatched {
                        removed_dispatched += 1;
                    }
                    extracted.push(value);
                }
                Err(other) => remaining.push(other),
            }
        }
        self.terminal_events = remaining;
        self.events_dispatched_up_to = dispatched - removed_dispatched;
        extracted
    }

    /// Remove and return every pending event matching `pred`, keeping the
    /// dispatch index consistent (see [`EventBroker::extract`]).
    pub(crate) fn extract_matching(
        &mut self,
        mut pred: impl FnMut(&TerminalEvent) -> bool,
    ) -> Vec<TerminalEvent> {
        self.extract(|event| if pred(&event) { Ok(event) } else { Err(event) })
    }

    /// Evict the oldest terminal events when the queue exceeds the cap
    /// (ARC-006), shifting `events_dispatched_up_to` so observer dispatch
    /// stays consistent with the moved positions.
    pub(crate) fn cap_pending(&mut self) {
        let excess = self
            .terminal_events
            .len()
            .saturating_sub(MAX_TERMINAL_EVENTS);
        if excess == 0 {
            return;
        }
        self.terminal_events.drain(..excess);
        self.events_dispatched_up_to = self.events_dispatched_up_to.saturating_sub(excess);
    }

    /// Extract the not-yet-dispatched events plus a snapshot of the
    /// currently registered observers into an owned [`ObserverDispatchBatch`],
    /// advancing the dispatch index so the same events aren't re-dispatched.
    /// Pure bookkeeping — no observer callback runs here, so this is safe to
    /// call while holding an exclusive lock.
    pub(crate) fn take_dispatch_batch(&mut self) -> ObserverDispatchBatch {
        if self.observers.is_empty() || self.terminal_events.is_empty() {
            return ObserverDispatchBatch::default();
        }

        let start = self.events_dispatched_up_to;
        if start >= self.terminal_events.len() {
            return ObserverDispatchBatch::default();
        }

        let events = self.terminal_events[start..].to_vec();
        let observers = self
            .observers
            .iter()
            .map(|entry| entry.observer.clone())
            .collect();
        self.events_dispatched_up_to = self.terminal_events.len();

        ObserverDispatchBatch { events, observers }
    }
}

/// A batch of terminal events plus a snapshot of the observers interested in
/// them, extracted from a `Terminal` for deferred delivery.
///
/// Building a batch ([`crate::terminal::Terminal::process_deferred`]) only
/// touches internal bookkeeping (owned clones, an index bump) and is
/// fast/non-blocking. [`ObserverDispatchBatch::deliver`] performs the actual
/// observer callbacks — which may be slow or re-entrant (e.g.
/// `PyCallbackObserver` re-entering Python under the GIL) — and is designed
/// to be called *without* holding any exclusive lock on the originating
/// `Terminal` (see ARC-001: observer dispatch must not run while a
/// `PtySession`'s `RwLock<Terminal>` write guard is held, or every concurrent
/// reader stalls behind it).
#[derive(Default)]
pub struct ObserverDispatchBatch {
    events: Vec<TerminalEvent>,
    observers: Vec<Arc<dyn TerminalObserver>>,
}

impl ObserverDispatchBatch {
    /// True if there is nothing to deliver (no observers, or no new events).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() || self.observers.is_empty()
    }

    /// Deliver the batch to observers.
    ///
    /// Operates purely on the owned snapshot captured when the batch was
    /// created — no `Terminal` borrow is held during this call, so it is
    /// safe to invoke after dropping a `RwLock`/`Mutex` guard around the
    /// `Terminal` that produced it.
    ///
    /// Mirrors the panic-isolation behavior of the old inline dispatch
    /// (ARC-007): a panicking observer is caught and logged rather than
    /// unwinding through the caller.
    pub fn deliver(self) {
        for event in &self.events {
            let category = crate::observer::event_category(event);
            let event_kind = event.kind();
            for observer in &self.observers {
                // Check subscriptions
                if let Some(subs) = observer.subscriptions() {
                    if !subs.contains(&event_kind) {
                        continue;
                    }
                }

                // ARC-007: isolate observer panics. A panicking observer (e.g. a
                // misbehaving Python callback via PyCallbackObserver) must not
                // unwind through the caller — catch the panic, log, and continue
                // with the remaining observers/events.
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match category {
                        crate::observer::EventCategory::Zone => observer.on_zone_event(event),
                        crate::observer::EventCategory::Command => observer.on_command_event(event),
                        crate::observer::EventCategory::Environment => {
                            observer.on_environment_event(event)
                        }
                        crate::observer::EventCategory::Screen => observer.on_screen_event(event),
                    }
                    observer.on_event(event);
                }))
                .is_err();
                if panicked {
                    log::error!(
                        target: "par_term_emu_core_rust::terminal::event_broker",
                        "par-term-emu: terminal observer panicked during dispatch; \
                         isolating to keep Terminal state consistent (ARC-007)"
                    );
                }
            }
        }
    }
}
