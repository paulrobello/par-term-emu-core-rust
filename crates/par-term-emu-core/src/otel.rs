//! Shared OpenTelemetry OTLP plumbing for the workspace's opt-in metrics
//! layers (ARC-001): the wire grammar ([`OtelProtocol`], header parsing), the
//! provider lifecycle ([`init`] / [`OtelGuard`]), and the stderr-reporting
//! exporter. The per-binary instrument facades stay with their consumers —
//! `src/streaming/metrics/` (the streamer's `ptec.streamer.*` series) and
//! `crates/par-mux/src/mux/metrics/` (the daemon's `ptec.mux.*` series) — as
//! thin per-binary config over this layer; before the extraction both carried
//! hand-maintained near-clones of everything below (the ~1,100-line
//! duplication the audit flagged, with drift already visible).
//!
//! Gate discipline (kanban 01a18331, following par-rt-db's ENH-018):
//!
//! * **Compile-time:** the `otel` cargo feature (default off). The grammar
//!   below is plain std and always compiles; the SDK-backed lifecycle compiles
//!   only with the feature, and without it [`init`] is a stub returning
//!   `None`, so a feature-less build pulls no OpenTelemetry code or
//!   dependencies.
//! * **Runtime:** each binary's own gate (`PAR_TERM_STREAMER_OTEL_ENABLED` /
//!   `PAR_MUX_OTEL_ENABLED`, or `--otel-enabled`) resolves in its clap surface
//!   and arrives here as the `enabled` flag.
//!
//! Instruments are created from `opentelemetry::global::meter(...)`, so when
//! no provider is installed the instruments are the SDK's no-op instruments:
//! record calls stay cheap and nothing is ever exported.

#[cfg(feature = "otel")]
mod provider;
#[cfg(not(feature = "otel"))]
mod stub;

// The lifecycle entry points are public for the two binaries' `metrics`
// facades to re-export; each binary keeps its own `init` wrapper that passes
// its name as the stderr-diagnostic label.
#[cfg(feature = "otel")]
pub use provider::{init, OtelGuard};
#[cfg(not(feature = "otel"))]
pub use stub::{init, OtelGuard};

/// OTLP exporter transport. [`OtelProtocol::Grpc`] is the historical default
/// (a plain `host:port` collector); [`OtelProtocol::Http`] posts protobuf to
/// the full endpoint URL, which is what CF-Access-fronted collectors like
/// par-obs's obs-ingest expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtelProtocol {
    /// gRPC (tonic) OTLP — the default, pointing at a plain `host:port` collector.
    Grpc,
    /// HTTP protobuf — POSTs to the full endpoint URL (CF-Access-fronted collectors).
    Http,
}

impl OtelProtocol {
    /// Parse the user-facing spelling (`grpc` | `http`, case-insensitive).
    pub fn parse(raw: &str) -> Option<OtelProtocol> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "grpc" => Some(OtelProtocol::Grpc),
            "http" => Some(OtelProtocol::Http),
            _ => None,
        }
    }
}

/// Parse a newline-separated `Key: Value` header list (the same shape as
/// par-mesh's `PAR_MESH_OTLP_HEADERS`). Blank lines and lines without a
/// `Key: Value` pair are skipped; parsed values are never logged.
pub fn parse_otlp_headers(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key.to_string(), value.to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_otlp_headers_skips_blanks_and_bare_lines() {
        let raw = "CF-Access-Client-Id: id@pardev.net\n\nnot a header\n  Authorization: Bearer tok  \nCF-Access-Client-Secret:\nX-A: 1\n";
        let parsed = parse_otlp_headers(raw);
        assert_eq!(
            parsed,
            vec![
                (
                    "CF-Access-Client-Id".to_string(),
                    "id@pardev.net".to_string()
                ),
                ("Authorization".to_string(), "Bearer tok".to_string()),
                ("X-A".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn otel_protocol_parse_is_case_insensitive_and_total() {
        assert_eq!(OtelProtocol::parse("HTTP"), Some(OtelProtocol::Http));
        assert_eq!(OtelProtocol::parse(" grpc "), Some(OtelProtocol::Grpc));
        assert_eq!(OtelProtocol::parse("carrier-pigeon"), None);
    }
}
