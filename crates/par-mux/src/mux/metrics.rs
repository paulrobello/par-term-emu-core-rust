//! Env-gated OpenTelemetry metrics for the par-mux daemon (kanban 01a18d2606).
//!
//! Two gates, the streamer's discipline (kanban 01a18331, itself following
//! par-rt-db's ENH-018):
//!
//! * **Compile-time:** the `otel` cargo feature (default off). Without it
//!   the instrument facade compiles to an identical-API no-op stub, so the
//!   default build pulls in no OpenTelemetry code or dependencies.
//! * **Runtime:** `PAR_MUX_OTEL_ENABLED` (or `--otel-enabled`) on the
//!   daemon. A feature-built binary makes zero OTLP calls unless the
//!   operator opts in, and [`init`] installs the SDK's global meter
//!   provider in the serving process before the tree restore spawns any
//!   pane, so every instrument binds to it.
//!
//! The OTLP plumbing itself (protocol grammar, header parsing, provider
//! lifecycle, reporting exporter) is the shared `par_term_emu_core::otel`
//! layer (ARC-001) — the hand-maintained mirror this module used to carry is
//! gone; this module re-exports the shared surface unchanged and keeps only
//! the daemon-specific parts: the instrument facade below and [`init`]'s
//! label binding plus the `daemon_starts` bump.
//!
//! Instruments are created from `opentelemetry::global::meter(...)`, so when
//! no provider is installed the instruments are the SDK's no-op instruments:
//! record calls stay cheap and nothing is ever exported. Series names use the
//! `ptec.mux.*` prefix so par-obs Grafana can panel them untransformed.

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

/// Install the SDK's global meter provider when `enabled` — the daemon
/// resolves `--otel-enabled` / `PAR_MUX_OTEL_ENABLED` via clap's `env =`
/// attributes and passes the values here. Called once from the daemon's
/// bind, before the tree restore spawns any pane, so every instrument binds
/// to the real provider. Returns the guard holding the provider — thread it
/// into the serving half and shut it down explicitly after the serve loop
/// returns. Thin per-binary wrapper over the shared layer, binding the
/// daemon's stderr-diagnostic label and recording the daemon start.
pub fn init(
    enabled: bool,
    protocol: OtelProtocol,
    endpoint: &str,
    headers: Vec<(String, String)>,
    service_name: &str,
) -> Option<OtelGuard> {
    let guard = par_term_emu_core::otel::init(
        enabled,
        protocol,
        endpoint,
        headers,
        service_name,
        "par-mux",
    )?;
    Metrics::global().daemon_started();
    Some(guard)
}
