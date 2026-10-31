//! The real `otel` build: instruments bound to the global meter provider.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, UpDownCounter};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::Temporality;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// The daemon's instruments, bound to the global meter provider current at
/// construction. In the daemon binary [`init`] installs the provider before
/// any instrument is created; embedders never install one, so instruments
/// bind to the SDK's no-op provider and record calls are cheap.
pub(crate) struct Metrics {
    sessions: UpDownCounter<i64>,
    panes: UpDownCounter<i64>,
    connected_clients: UpDownCounter<i64>,
    daemon_starts: Counter<u64>,
    client_connections: Counter<u64>,
    client_attaches: Counter<u64>,
    /// The last synced value behind each gauge: [`Metrics::sync_gauges`]
    /// adds only the delta, so the cumulative sum always reads the live
    /// count and a synced-twice value exports once.
    last_sessions: AtomicI64,
    last_panes: AtomicI64,
    last_connected: AtomicI64,
}

impl Metrics {
    /// Bind instruments to the current global meter provider. Test/bench
    /// callers construct fresh instances after installing their own provider
    /// instead of going through [`Metrics::global`], whose OnceLock caches
    /// the first binding for the process lifetime.
    pub(crate) fn new() -> Metrics {
        let meter = global::meter("par-mux");
        Metrics {
            sessions: meter
                .i64_up_down_counter("ptec.mux.sessions")
                .with_description("Sessions currently in the daemon's tree (restores included)")
                .build(),
            panes: meter
                .i64_up_down_counter("ptec.mux.panes")
                .with_description("Panes currently in the daemon's tree (restores included)")
                .build(),
            connected_clients: meter
                .i64_up_down_counter("ptec.mux.connected_clients")
                .with_description("Control-socket clients currently registered for broadcasts")
                .build(),
            daemon_starts: meter
                .u64_counter("ptec.mux.daemon_starts")
                .with_description("Daemon process starts with metrics enabled")
                .build(),
            client_connections: meter
                .u64_counter("ptec.mux.client_connections")
                .with_description("Control-socket connections accepted")
                .build(),
            client_attaches: meter
                .u64_counter("ptec.mux.client_attaches")
                .with_description("Clients that registered their view (%client-attached)")
                .build(),
            last_sessions: AtomicI64::new(0),
            last_panes: AtomicI64::new(0),
            last_connected: AtomicI64::new(0),
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
    /// Fold the current live counts into the up/down gauges. The daemon's
    /// cadence loop calls this once a second; only the delta since the last
    /// sync is added, so the cumulative sum always reads the live count.
    /// Sync instead of transactional ±1 keeps the gauges restore-proof:
    /// after a daemon restart the first sync corrects each series in one
    /// step, and no lifecycle call site can be missed.
    pub(crate) fn sync_gauges(&self, sessions: i64, panes: i64, connected_clients: i64) {
        let last_sessions = self.last_sessions.swap(sessions, Ordering::AcqRel);
        let last_panes = self.last_panes.swap(panes, Ordering::AcqRel);
        let last_connected = self
            .last_connected
            .swap(connected_clients, Ordering::AcqRel);
        self.sessions.add(sessions - last_sessions, &[]);
        self.panes.add(panes - last_panes, &[]);
        self.connected_clients
            .add(connected_clients - last_connected, &[]);
    }

    pub(crate) fn daemon_started(&self) {
        self.daemon_starts.add(1, &[]);
    }

    pub(crate) fn client_connected(&self) {
        self.client_connections.add(1, &[]);
    }

    pub(crate) fn client_attached(&self) {
        self.client_attaches.add(1, &[]);
    }
}

/// Holds the SDK meter provider for the process lifetime; dropping it flushes
/// and shuts the exporter down. Call [`OtelGuard::shutdown`] explicitly while
/// the process's async runtime is still alive — the drop path can lose the
/// final export window (the short-lived-smoke report).
pub struct OtelGuard {
    provider: opentelemetry_sdk::metrics::SdkMeterProvider,
}

impl OtelGuard {
    /// Flush and shut the provider down now, while the process's async
    /// runtime is alive. Non-fatal when it fails (the process is exiting
    /// anyway); the drop path remains the last resort.
    pub fn shutdown(&self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("par-term-streamer: OpenTelemetry shutdown failed ({e})");
        }
    }
}

/// Delegating exporter that surfaces export failures on stderr. The 0.33 SDK
/// compiles its `otel_error!` reporting out by default (`internal-logs` off),
/// so a misconfigured endpoint or a dead collector failed silently — a smoke
/// run once claimed "exports shipped" while its series never landed. The
/// first consecutive failure and every 30th after print; a success resets.
struct ReportingExporter<E> {
    inner: E,
    consecutive_failures: std::sync::atomic::AtomicU64,
}

impl<E: PushMetricExporter> ReportingExporter<E> {
    /// Whether the n-th consecutive failure (0-based) prints. Exposed for
    /// the rate-limit test.
    fn should_report(n: u64) -> bool {
        n.is_multiple_of(30)
    }
}

impl<E: PushMetricExporter> PushMetricExporter for ReportingExporter<E> {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        let result = self.inner.export(metrics).await;
        if let Err(e) = &result {
            let n = self
                .consecutive_failures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if Self::should_report(n) {
                eprintln!(
                    "par-mux: OTLP metrics export failed (consecutive #{}, will retry): {e}",
                    n + 1
                );
            }
        } else {
            self.consecutive_failures
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        result
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn temporality(&self) -> Temporality {
        self.inner.temporality()
    }
}

/// Install the SDK's global meter provider when `enabled` — the daemon
/// resolves `--otel-enabled` / `PAR_MUX_OTEL_ENABLED` via clap's `env =`
/// attributes and passes the values here. Called once from the daemon's
/// bind, before the tree restore spawns any pane, so every instrument
/// binds to the real provider. Returns the guard holding the provider —
/// thread it into the serving half and shut it down explicitly after the
/// serve loop returns.
pub fn init(
    enabled: bool,
    protocol: crate::mux::metrics::OtelProtocol,
    endpoint: &str,
    headers: Vec<(String, String)>,
    service_name: &str,
) -> Option<OtelGuard> {
    use crate::mux::metrics::OtelProtocol;

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
            eprintln!("par-mux: OpenTelemetry exporter init failed ({e}); metrics disabled");
            return None;
        }
    };

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name.to_string())
        .build();

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(ReportingExporter {
        inner: exporter,
        consecutive_failures: std::sync::atomic::AtomicU64::new(0),
    })
    .with_interval(Duration::from_secs(15))
    .build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider.clone());
    Metrics::global().daemon_started();
    Some(OtelGuard { provider })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::data::Metric;

    /// A stub exporter whose `export` result is scripted: the reporting
    /// wrapper's failure accounting is what the tests pin.
    struct StubExporter {
        fail: bool,
        exports: std::sync::atomic::AtomicU64,
    }

    impl StubExporter {
        fn failing() -> StubExporter {
            StubExporter {
                fail: true,
                exports: std::sync::atomic::AtomicU64::new(0),
            }
        }

        fn succeeding() -> StubExporter {
            StubExporter {
                fail: false,
                exports: std::sync::atomic::AtomicU64::new(0),
            }
        }
    }

    impl PushMetricExporter for StubExporter {
        async fn export(&self, _metrics: &ResourceMetrics) -> OTelSdkResult {
            self.exports
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail {
                Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                    "boom".to_string(),
                ))
            } else {
                Ok(())
            }
        }

        fn force_flush(&self) -> OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Ok(())
        }

        fn temporality(&self) -> Temporality {
            Temporality::Cumulative
        }
    }

    /// The first consecutive failure prints; the next 29 stay quiet; the
    /// 31st prints again (every 30th after the first).
    #[test]
    fn export_failure_report_is_rate_limited() {
        assert!(ReportingExporter::<StubExporter>::should_report(0));
        assert!(!ReportingExporter::<StubExporter>::should_report(1));
        assert!(!ReportingExporter::<StubExporter>::should_report(29));
        assert!(ReportingExporter::<StubExporter>::should_report(30));
    }

    /// A failed export bumps the consecutive-failure count; a success
    /// resets it.
    #[test]
    fn reporting_exporter_counts_failures_and_resets_on_success() {
        // The stub exporter's futures are ready on their first poll, so a
        // no-op-waker poll loop stands in for a runtime (the member has no
        // tokio dev-dependency).
        fn block_on<F: std::future::Future>(fut: F) -> F::Output {
            use std::task::{Context, Poll, Waker};
            let mut fut = std::pin::pin!(fut);
            let mut cx = Context::from_waker(Waker::noop());
            loop {
                if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                    return v;
                }
                std::thread::yield_now();
            }
        }
        block_on(async {
            let stub = StubExporter::failing();
            let wrapper = ReportingExporter {
                inner: stub,
                consecutive_failures: std::sync::atomic::AtomicU64::new(0),
            };
            let rm = ResourceMetrics::default();
            wrapper.export(&rm).await.unwrap_err();
            wrapper.export(&rm).await.unwrap_err();
            assert_eq!(
                wrapper
                    .consecutive_failures
                    .load(std::sync::atomic::Ordering::SeqCst),
                2,
                "both failures counted"
            );

            let wrapper = ReportingExporter {
                inner: StubExporter::succeeding(),
                consecutive_failures: std::sync::atomic::AtomicU64::new(7),
            };
            wrapper.export(&rm).await.unwrap();
            assert_eq!(
                wrapper
                    .consecutive_failures
                    .load(std::sync::atomic::Ordering::SeqCst),
                0,
                "success resets the count"
            );
        })
    }

    /// The full instrument surface exports the six series through an
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
        m.sync_gauges(3, 7, 2);
        m.sync_gauges(3, 6, 3);
        m.daemon_started();
        m.client_connected();
        m.client_attached();

        provider.force_flush().unwrap();
        let finished = exporter.get_finished_metrics().unwrap();

        let mut names = std::collections::HashSet::new();
        let mut sessions = 0.0f64;
        let mut panes = 0.0f64;
        let mut connected = 0.0f64;
        let mut starts = 0.0f64;
        let mut connections = 0.0f64;
        let mut attaches = 0.0f64;
        for rm in &finished {
            for sm in rm.scope_metrics() {
                for metric in sm.metrics() {
                    names.insert(metric.name().to_string());
                    match metric.name() {
                        "ptec.mux.sessions" => sum_of(metric, &mut sessions),
                        "ptec.mux.panes" => sum_of(metric, &mut panes),
                        "ptec.mux.connected_clients" => sum_of(metric, &mut connected),
                        "ptec.mux.daemon_starts" => sum_of(metric, &mut starts),
                        "ptec.mux.client_connections" => sum_of(metric, &mut connections),
                        "ptec.mux.client_attaches" => sum_of(metric, &mut attaches),
                        _ => {}
                    }
                }
            }
        }

        for name in [
            "ptec.mux.sessions",
            "ptec.mux.panes",
            "ptec.mux.connected_clients",
            "ptec.mux.daemon_starts",
            "ptec.mux.client_connections",
            "ptec.mux.client_attaches",
        ] {
            assert!(
                names.contains(name),
                "missing series {name}; exported {names:?}"
            );
        }
        assert_eq!(sessions, 3.0, "sessions gauge counts from zero");
        assert_eq!(panes, 6.0, "panes gauge: 7 then -1 delta");
        assert_eq!(connected, 3.0, "connected gauge: 2 then +1 delta");
        assert_eq!(starts, 1.0, "daemon_starts counter");
        assert_eq!(connections, 1.0, "client_connections counter");
        assert_eq!(attaches, 1.0, "client_attaches counter");
    }

    fn sum_of(metric: &Metric, out: &mut f64) {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        match metric.data() {
            AggregatedMetrics::I64(MetricData::Sum(sum)) => {
                for p in sum.data_points() {
                    *out += p.value() as f64;
                }
            }
            AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                for p in sum.data_points() {
                    *out += p.value() as f64;
                }
            }
            _ => {}
        }
    }
}
