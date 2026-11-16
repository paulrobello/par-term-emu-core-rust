//! No-op stub: identical call surface to the otel module, no OTel code.
//!
//! The lifecycle stub ([`super::init`] returning `None`, the marker
//! [`super::OtelGuard`]) lives in the shared `par_term_emu_core::otel` layer;
//! this file carries only the instrument facade.

#![allow(unused_variables)]

/// Metrics facade for builds without the `otel` feature: a unit struct whose
/// methods are all no-ops. Same surface as the otel module's `Metrics`.
pub(crate) struct Metrics;

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
