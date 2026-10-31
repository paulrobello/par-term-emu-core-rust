//! No-op stub: identical call surface to the otel module, no OTel code.

#![allow(unused_variables)]

/// Metrics facade for builds without the `otel` feature: a unit struct whose
/// methods are all no-ops. Same surface as the otel module's `Metrics`.
pub(crate) struct Metrics;

/// Marker returned by [`init`] when the feature is compiled out.
pub struct OtelGuard;

impl OtelGuard {
    /// Surface parity with the otel module: the stub carries no provider,
    /// so an explicit shutdown is a no-op (main calls it unconditionally).
    pub fn shutdown(&self) {}
}

impl Metrics {
    /// The stub is a unit struct shared by every caller.
    pub(crate) fn global() -> &'static Metrics {
        static GLOBAL: Metrics = Metrics;
        &GLOBAL
    }
}

impl Metrics {
    #[allow(dead_code, reason = "surface parity with the otel module")]
    pub(crate) fn new() -> Metrics {
        Metrics
    }

    /// Fold the current live counts into the up/down gauges. The daemon's
    /// cadence loop calls this once a second; the facade adds only the delta
    /// since the last sync, so a value synced twice is exported once. Sync
    /// instead of transactional ±1 keeps the gauges restore-proof: after a
    /// restart, the first sync corrects the whole series in one step.
    #[allow(dead_code, reason = "surface parity with the otel module")]
    pub(crate) fn sync_gauges(&self, _sessions: i64, _panes: i64, _connected_clients: i64) {}

    /// Recorded from the daemon binary at startup; a no-op unless the otel
    /// feature is compiled in.
    #[allow(dead_code, reason = "surface parity with the otel module")]
    pub(crate) fn daemon_started(&self) {}

    /// One control-socket connection accepted.
    #[allow(dead_code, reason = "surface parity with the otel module")]
    pub(crate) fn client_connected(&self) {}

    /// One `%client-attached` announcement (a client registered its view).
    #[allow(dead_code, reason = "surface parity with the otel module")]
    pub(crate) fn client_attached(&self) {}
}

/// Runtime gate: with the feature compiled out, metrics are always off and
/// there is nothing to construct or shut down. Signature parity with the
/// otel module's [`init`].
pub fn init(
    _enabled: bool,
    _protocol: crate::mux::metrics::OtelProtocol,
    _endpoint: &str,
    _headers: Vec<(String, String)>,
    _service_name: &str,
) -> Option<OtelGuard> {
    None
}
