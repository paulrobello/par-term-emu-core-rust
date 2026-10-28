//! The real `otel` build: instruments bound to the global meter provider.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use std::sync::OnceLock;
use std::time::Duration;

/// Streamer metrics instruments, bound to the global meter provider current
/// at construction. In the streamer binary [`init`] installs the
/// provider before any instrument is created; embedders never install one, so
/// instruments bind to the SDK's no-op provider and record calls are cheap.
pub(crate) struct Metrics {
    connected_clients: UpDownCounter<i64>,
    active_sessions: UpDownCounter<i64>,
    send_latency: Histogram<f64>,
    encode_latency: Histogram<f64>,
    connections: Counter<u64>,
    reconnects: Counter<u64>,
    mux_socket_errors: Counter<u64>,
}

impl Metrics {
    /// Bind instruments to the current global meter provider. Test/bench
    /// callers construct fresh instances after installing their own provider
    /// instead of going through [`Metrics::global`], whose OnceLock caches
    /// the first binding for the process lifetime.
    pub(crate) fn new() -> Metrics {
        let meter = global::meter("par-term-streamer");
        Metrics {
            connected_clients: meter
                .i64_up_down_counter("ptec.streamer.connected_clients")
                .with_description("WebSocket clients currently connected to the streamer")
                .build(),
            active_sessions: meter
                .i64_up_down_counter("ptec.streamer.active_sessions")
                .with_description("Streaming sessions currently registered")
                .build(),
            send_latency: meter
                .f64_histogram("ptec.streamer.event_send_latency")
                .with_description(
                    "Milliseconds from dequeue of a pane event to written on the client socket",
                )
                .with_unit("ms")
                .build(),
            encode_latency: meter
                .f64_histogram("ptec.streamer.protobuf_encode_latency")
                .with_description("Milliseconds to encode one ServerMessage to protobuf")
                .with_unit("ms")
                .build(),
            connections: meter
                .u64_counter("ptec.streamer.client_connections")
                .with_description("Accepted client connections")
                .build(),
            reconnects: meter
                .u64_counter("ptec.streamer.client_reconnects")
                .with_description(
                    "Connections to a session whose previous client left within the reconnect window",
                )
                .build(),
            mux_socket_errors: meter
                .u64_counter("ptec.streamer.mux_socket_errors")
                .with_description("Errors on the par-mux control socket, by operation")
                .build(),
        }
    }

    /// Process-wide instruments. In the streamer binary the first call
    /// happens after `init` installed the real provider.
    pub(crate) fn global() -> &'static Metrics {
        static GLOBAL: OnceLock<Metrics> = OnceLock::new();
        GLOBAL.get_or_init(Metrics::new)
    }
}

impl Metrics {
    pub(crate) fn client_connected(&self, session: &str, transport: &str, reconnect: bool) {
        let session_kv = KeyValue::new("session", session.to_string());
        self.connected_clients
            .add(1, std::slice::from_ref(&session_kv));
        self.connections.add(
            1,
            &[
                session_kv.clone(),
                KeyValue::new("transport", transport.to_string()),
            ],
        );
        if reconnect {
            self.reconnects.add(1, &[session_kv]);
        }
    }

    pub(crate) fn client_disconnected(&self, session: &str) {
        self.connected_clients
            .add(-1, &[KeyValue::new("session", session.to_string())]);
    }

    pub(crate) fn session_opened(&self) {
        self.active_sessions.add(1, &[]);
    }

    pub(crate) fn session_closed(&self) {
        self.active_sessions.add(-1, &[]);
    }

    pub(crate) fn record_send_latency_ms(&self, ms: f64) {
        self.send_latency.record(ms, &[]);
    }

    pub(crate) fn record_encode_latency_ms(&self, ms: f64) {
        self.encode_latency.record(ms, &[]);
    }

    /// Recorded from the mux session factory (feature `mux`); lints dead in
    /// streaming-only builds.
    #[cfg_attr(not(feature = "mux"), allow(dead_code))]
    pub(crate) fn mux_socket_error(&self, operation: &str) {
        self.mux_socket_errors
            .add(1, &[KeyValue::new("operation", operation.to_string())]);
    }
}

/// Holds the SDK meter provider for the process lifetime; dropping it flushes
/// and shuts the exporter down.
pub struct OtelGuard {
    #[allow(dead_code, reason = "RAII guard: the field exists to be dropped")]
    provider: opentelemetry_sdk::metrics::SdkMeterProvider,
}

/// Install the SDK's global meter provider when `enabled` — the binary
/// resolves `--otel-enabled` / `PAR_TERM_STREAMER_OTEL_ENABLED` via clap's
/// `env =` attributes and passes the values here. Called once from the
/// streamer binary's main, before the server starts, so every instrument
/// binds to the real provider. Returns the guard holding the provider — bind
/// it in main for the process lifetime.
pub fn init(
    enabled: bool,
    protocol: crate::streaming::metrics::OtelProtocol,
    endpoint: &str,
    headers: Vec<(String, String)>,
    service_name: &str,
) -> Option<OtelGuard> {
    use crate::streaming::metrics::OtelProtocol;

    if !enabled {
        return None;
    }

    let built = match protocol {
        OtelProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(Duration::from_secs(5))
            .build(),
        OtelProtocol::Http => {
            use opentelemetry_otlp::WithHttpConfig;
            let header_map: std::collections::HashMap<String, String> =
                headers.into_iter().collect();
            opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .with_headers(header_map)
                .with_timeout(Duration::from_secs(5))
                .build()
        }
    };
    let exporter = match built {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "par-term-streamer: OpenTelemetry exporter init failed ({e}); metrics disabled"
            );
            return None;
        }
    };

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name.to_string())
        .build();

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(15))
        .build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider.clone());
    Some(OtelGuard { provider })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::data::Metric;

    /// The full instrument surface exports the seven series through an
    /// in-memory exporter with the expected values. Deliberately one test:
    /// `set_meter_provider` is process-global and last-one-wins, so parallel
    /// otel tests would cross-contaminate each other.
    #[test]
    fn otel_series_export_through_in_memory_exporter() {
        let exporter = opentelemetry_sdk::metrics::InMemoryMetricExporter::default();
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter.clone()).build();
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(reader)
            .build();
        opentelemetry::global::set_meter_provider(provider.clone());

        let m = Metrics::new();
        m.session_opened();
        m.client_connected("s1", "test", false);
        m.client_connected("s1", "test", true);
        m.record_send_latency_ms(1.5);
        m.record_encode_latency_ms(0.25);
        m.mux_socket_error("cannot connect");
        m.client_disconnected("s1");
        m.session_closed();

        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();

        let mut names = std::collections::HashSet::new();
        let mut connections = 0.0f64;
        let mut reconnects = 0.0f64;
        let mut mux_errors = 0.0f64;
        let mut send_count = 0u64;
        let mut encode_count = 0u64;
        for rm in &finished {
            for sm in rm.scope_metrics() {
                for metric in sm.metrics() {
                    names.insert(metric.name().to_string());
                    match metric.name() {
                        "ptec.streamer.client_connections" => sum_of(metric, &mut connections),
                        "ptec.streamer.client_reconnects" => sum_of(metric, &mut reconnects),
                        "ptec.streamer.mux_socket_errors" => sum_of(metric, &mut mux_errors),
                        "ptec.streamer.event_send_latency" => hist_count(metric, &mut send_count),
                        "ptec.streamer.protobuf_encode_latency" => {
                            hist_count(metric, &mut encode_count)
                        }
                        _ => {}
                    }
                }
            }
        }

        for name in [
            "ptec.streamer.connected_clients",
            "ptec.streamer.active_sessions",
            "ptec.streamer.event_send_latency",
            "ptec.streamer.protobuf_encode_latency",
            "ptec.streamer.client_connections",
            "ptec.streamer.client_reconnects",
            "ptec.streamer.mux_socket_errors",
        ] {
            assert!(
                names.contains(name),
                "missing series {name}; exported {names:?}"
            );
        }
        assert_eq!(connections, 2.0, "connections counter");
        assert_eq!(reconnects, 1.0, "reconnects counter");
        assert_eq!(mux_errors, 1.0, "mux error counter");
        assert_eq!(send_count, 1, "send-latency histogram");
        assert_eq!(encode_count, 1, "encode-latency histogram");
    }

    fn sum_of(metric: &Metric, out: &mut f64) {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        match metric.data() {
            AggregatedMetrics::I64(m) => {
                if let MetricData::Sum(sum) = m {
                    for p in sum.data_points() {
                        *out += p.value() as f64;
                    }
                }
            }
            AggregatedMetrics::U64(m) => {
                if let MetricData::Sum(sum) = m {
                    for p in sum.data_points() {
                        *out += p.value() as f64;
                    }
                }
            }
            _ => {}
        }
    }

    fn hist_count(metric: &Metric, out: &mut u64) {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        match metric.data() {
            AggregatedMetrics::F64(m) => {
                if let MetricData::Histogram(h) = m {
                    for p in h.data_points() {
                        *out += p.count();
                    }
                }
            }
            AggregatedMetrics::U64(m) => {
                if let MetricData::Histogram(h) = m {
                    for p in h.data_points() {
                        *out += p.count();
                    }
                }
            }
            _ => {}
        }
    }
}
