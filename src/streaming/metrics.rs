//! Env-gated OpenTelemetry metrics for the streaming server (kanban 01a18331).
//!
//! Two gates, following par-rt-db's ENH-018 discipline:
//!
//! * **Compile-time:** the `otel` cargo feature (default off). Without it
//!   the instrument facade compiles to an identical-API no-op stub, so the
//!   default build pulls in no OpenTelemetry code or dependencies.
//! * **Runtime:** `PAR_TERM_STREAMER_OTEL_ENABLED` in the streamer binary.
//!   A feature-built binary makes zero OTLP calls unless the operator opts
//!   in, and [`init`] installs the SDK's global meter provider
//!   before the server starts so every instrument binds to it.
//!
//! The OTLP plumbing itself (protocol grammar, header parsing, provider
//! lifecycle, reporting exporter) is the shared `par_term_emu_core::otel`
//! layer (ARC-001); this module re-exports its public surface unchanged and
//! keeps only the streamer-specific parts — the instrument facade below and
//! [`init`]'s label binding.
//!
//! Instruments are created from `opentelemetry::global::meter(...)`, so when
//! no provider is installed the instruments are the SDK's no-op instruments:
//! record calls stay cheap and nothing is ever exported. Series names use the
//! `ptec.streamer.*` prefix so par-obs Grafana can panel them untransformed.

#[cfg(feature = "otel")]
mod otel;
#[cfg(not(feature = "otel"))]
mod stub;

// The instrument facade stays crate-internal; the shared lifecycle surface
// and grammar keep their long-standing public paths through this module.
#[cfg(feature = "otel")]
pub(crate) use otel::Metrics;
#[cfg(not(feature = "otel"))]
pub(crate) use stub::Metrics;

pub use par_term_emu_core::otel::{parse_otlp_headers, OtelGuard, OtelProtocol};

/// A connection counts as a reconnect when the session's previous client left
/// within this window (ms). The wire protocol has no stable client identity —
/// every connection gets a fresh UUID — so the session is the identity, and
/// the heuristic is documented rather than hidden.
pub(crate) const RECONNECT_WINDOW_MS: u64 = 300_000;

/// Install the SDK's global meter provider when `enabled` — the streamer
/// binary resolves `--otel-enabled` / `PAR_TERM_STREAMER_OTEL_ENABLED` via
/// clap's `env =` attributes and passes the values here. Called once from
/// the streamer binary's main, before the server starts, so every instrument
/// binds to the real provider. Returns the guard holding the provider — bind
/// it in main for the process lifetime. Thin per-binary wrapper over the
/// shared layer, binding the streamer's stderr-diagnostic label.
pub fn init(
    enabled: bool,
    protocol: OtelProtocol,
    endpoint: &str,
    headers: Vec<(String, String)>,
    service_name: &str,
) -> Option<OtelGuard> {
    par_term_emu_core::otel::init(
        enabled,
        protocol,
        endpoint,
        headers,
        service_name,
        "par-term-streamer",
    )
}
