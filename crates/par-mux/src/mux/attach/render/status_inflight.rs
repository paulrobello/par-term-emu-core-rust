//! The seed/status snapshot coordination: at most one `client-snapshot`
//! in flight per session, with the facts the flight carries and the two
//! flags that outlive it — the stale mark and the pump-visible `End`.

/// The one `client-snapshot` a session may have owed, with its issuer's
/// role: the reply continuations branch on it (`apply_*_snapshot`), and
/// the seed role additionally collects the pushes handled while it was
/// owed ([`StatusInflight::note_seed_race`]). The role facts live in the
/// variants, so a seed flag with no seed flight, or a landing marker
/// with no re-seed, is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapshotFlight {
    /// The seed's (`seed_status_snapshot`, the session-file impl): pushes
    /// handled while it is owed join the raced set
    /// ([`StatusInflight::raced_panes`]).
    Seed {
        /// When the snapshot was written: the storm rule's abandon clock.
        since: std::time::Instant,
    },
    /// The pump's throttled re-query (`refresh_status_async`).
    Refresh {
        /// When the snapshot was written: the storm rule's abandon clock.
        since: std::time::Instant,
    },
    /// A re-seed's (`reseed_status_snapshot`, the navigate-file impl).
    Reseed {
        /// Whether the re-seed is the gone-view landing's (`land_gone_view`,
        /// both branches): its reply's `SessionGone` walks the interactive
        /// ladder directly — the async form of the sync path's `false`
        /// fall-through — instead of re-landing from the same stale cache
        /// forever.
        landing: bool,
        /// When the snapshot was written: the storm rule's abandon clock.
        since: std::time::Instant,
    },
}

impl SnapshotFlight {
    /// The issue stamp: the storm rule compares its age against
    /// `status::STATUS_TIMEOUT`.
    pub(super) fn since(self) -> std::time::Instant {
        match self {
            SnapshotFlight::Seed { since }
            | SnapshotFlight::Refresh { since }
            | SnapshotFlight::Reseed { since, .. } => since,
        }
    }

    /// The same flight stamped `ago` earlier — the tests' clock control
    /// for the abandon branch.
    #[cfg(test)]
    pub(super) fn backdated(self, ago: std::time::Duration) -> Self {
        let since = self.since() - ago;
        match self {
            SnapshotFlight::Seed { .. } => SnapshotFlight::Seed { since },
            SnapshotFlight::Refresh { .. } => SnapshotFlight::Refresh { since },
            SnapshotFlight::Reseed { landing, .. } => SnapshotFlight::Reseed { landing, since },
        }
    }
}

/// The seed/status coordination state: which snapshot is owed (the
/// enum above), whether the status state is stale and needs a re-query
/// before the next paint (the throttled re-query on agent/sessions
/// churn), the seed's raced panes, and the nothing-left-to-show `End`
/// raised by the snapshot reply's continuation (the reply cannot end
/// the view through a return value any more); the pump's status step
/// checks it.
#[derive(Debug)]
pub(super) struct StatusInflight {
    /// The owed snapshot, `None` when idle. At most one at a time: the
    /// issuers guard on this slot, and a dirty mark landing mid-flight
    /// stays set for the reply's application to re-issue on a later tick.
    pub(super) flight: Option<SnapshotFlight>,
    /// The status state is stale and needs a re-query before the next
    /// paint.
    pub(super) dirty: bool,
    /// Agent panes whose state pushes were handled while the seed's
    /// snapshot was in flight. The sync path drained its pre-refresh
    /// pushes and left them out of the baseline; the seed's continuation
    /// must too, or the snapshot's entry suppresses an identical
    /// rebroadcast on a record the push stream never made. Only a seed
    /// flight accumulates it ([`Self::note_seed_race`]), and its
    /// completion drains it.
    raced_panes: Vec<u32>,
    /// The nothing-left-to-show `End`: once set the pump's status step
    /// ends the view; nothing clears it.
    pub(super) end: bool,
}

impl StatusInflight {
    /// Fresh state: the seed marks the bar stale before its first query.
    pub(super) fn new() -> Self {
        Self {
            flight: None,
            dirty: true,
            raced_panes: Vec::new(),
            end: false,
        }
    }

    /// Open the slot to `flight`, clearing the dirty mark — the issue
    /// consumes it.
    pub(super) fn begin(&mut self, flight: SnapshotFlight) {
        self.dirty = false;
        self.flight = Some(flight);
    }

    /// The reply arrived: close the slot and drain the seed's raced
    /// panes (empty unless the flight was the seed's).
    pub(super) fn complete(&mut self) -> Vec<u32> {
        self.flight = None;
        std::mem::take(&mut self.raced_panes)
    }

    /// The storm rule's abandon: a reply still owed past
    /// `status::STATUS_TIMEOUT` is dropped in place — the stale bar
    /// stands, and clearing the dirty mark too leaves the re-query to
    /// the next poll tick or event instead of queuing behind the same
    /// slow reply every pump pass.
    pub(super) fn abandon(&mut self) {
        self.flight = None;
        self.dirty = false;
    }

    /// A push handled while the seed's snapshot is in flight joins the
    /// raced set. A push handled while any other flight (or none) is
    /// owed does not — the set exists only for the seed's baseline
    /// adoption.
    pub(super) fn note_seed_race(&mut self, pane: u32) {
        if !matches!(self.flight, Some(SnapshotFlight::Seed { .. })) {
            return;
        }
        if !self.raced_panes.contains(&pane) {
            self.raced_panes.push(pane);
        }
    }
}
