//! The real `otel` build: instruments bound to the global meter provider.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, UpDownCounter};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;

/// The daemon's instruments, bound to the global meter provider current at
/// construction. In the daemon binary [`super::init`] installs the provider
/// before any instrument is created; embedders never install one, so
/// instruments bind to the SDK's no-op provider and record calls are cheap.
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

    /// Process-wide instruments. In the daemon binary the first call happens
    /// after `init` installed the real provider.
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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::data::Metric;

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
