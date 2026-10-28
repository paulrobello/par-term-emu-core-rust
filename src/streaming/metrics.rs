//! Env-gated OpenTelemetry metrics for the streaming server (kanban 01a18331).
//!
//! Two gates, following par-rt-db's ENH-018 discipline:
//!
//! * **Compile-time:** the `otel` cargo feature (default off). Without it
//!   this module compiles to an identical-API no-op stub, so the default
//!   build pulls in no OpenTelemetry code or dependencies.
//! * **Runtime:** `PAR_TERM_STREAMER_OTEL_ENABLED` in the streamer binary.
//!   A feature-built binary makes zero OTLP calls unless the operator opts
//!   in, and [`init`] installs the SDK's global meter provider
//!   before the server starts so every instrument binds to it.
//!
//! Instruments are created from `opentelemetry::global::meter(...)`, so when
//! no provider is installed the instruments are the SDK's no-op instruments:
//! record calls stay cheap and nothing is ever exported. Series names use the
//! `ptec.streamer.*` prefix so par-obs Grafana can panel them untransformed.

#[cfg(feature = "otel")]
mod otel;
#[cfg(not(feature = "otel"))]
mod stub;

// `init` is the binary-facing entry point (public); the instrument
// facade itself stays crate-internal.
#[cfg(feature = "otel")]
pub(crate) use otel::Metrics;
#[cfg(feature = "otel")]
pub use otel::{init, OtelGuard};
#[cfg(not(feature = "otel"))]
pub(crate) use stub::Metrics;
#[cfg(not(feature = "otel"))]
pub use stub::{init, OtelGuard};

/// A connection counts as a reconnect when the session's previous client left
/// within this window (ms). The wire protocol has no stable client identity —
/// every connection gets a fresh UUID — so the session is the identity, and
/// the heuristic is documented rather than hidden.
pub(crate) const RECONNECT_WINDOW_MS: u64 = 300_000;

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
