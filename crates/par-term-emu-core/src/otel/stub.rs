//! No-op stub: the lifecycle surface without the `otel` feature, no OTel
//! code.

/// Marker returned by [`super::init`] when the feature is compiled out.
pub struct OtelGuard;

impl OtelGuard {
    /// Surface parity with the provider build: the stub carries no provider,
    /// so an explicit shutdown is a no-op (binaries call it unconditionally).
    pub fn shutdown(&self) {}
}

/// Runtime gate: with the feature compiled out, metrics are always off and
/// there is nothing to construct or shut down. Signature parity with the
/// provider build's [`super::init`].
pub fn init(
    _enabled: bool,
    _protocol: crate::otel::OtelProtocol,
    _endpoint: &str,
    _headers: Vec<(String, String)>,
    _service_name: &str,
    _label: &'static str,
) -> Option<OtelGuard> {
    None
}
