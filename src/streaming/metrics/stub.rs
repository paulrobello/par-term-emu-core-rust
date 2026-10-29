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

    pub(crate) fn client_connected(&self, session: &str, transport: &str, reconnect: bool) {}
    pub(crate) fn client_disconnected(&self, session: &str) {}
    pub(crate) fn session_opened(&self) {}
    pub(crate) fn session_closed(&self) {}
    pub(crate) fn record_send_latency_ms(&self, ms: f64) {}
    pub(crate) fn record_encode_latency_ms(&self, ms: f64) {}
    #[cfg_attr(not(feature = "mux"), allow(dead_code))]
    pub(crate) fn mux_socket_error(&self, operation: &str) {}
}

/// Runtime gate: with the feature compiled out, metrics are always off and
/// there is nothing to construct or shut down. Signature parity with the
/// otel module's [`init`].
pub fn init(
    _enabled: bool,
    _protocol: crate::streaming::metrics::OtelProtocol,
    _endpoint: &str,
    _headers: Vec<(String, String)>,
    _service_name: &str,
) -> Option<OtelGuard> {
    None
}
