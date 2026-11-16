//! The SDK-backed `otel` build: the provider lifecycle both binaries share.

use crate::otel::OtelProtocol;
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::metrics::Temporality;
use std::time::Duration;

/// Holds the SDK meter provider for the process lifetime; dropping it flushes
/// and shuts the exporter down. Call [`OtelGuard::shutdown`] explicitly while
/// the process's async runtime is still alive — the drop path can lose the
/// final export window (the short-lived-smoke report).
pub struct OtelGuard {
    provider: opentelemetry_sdk::metrics::SdkMeterProvider,
    label: &'static str,
}

impl OtelGuard {
    /// Flush and shut the provider down now, while the process's async
    /// runtime is alive. Non-fatal when it fails (the process is exiting
    /// anyway); the drop path remains the last resort.
    pub fn shutdown(&self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("{}: OpenTelemetry shutdown failed ({e})", self.label);
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
    label: &'static str,
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
                    "{}: OTLP metrics export failed (consecutive #{}, will retry): {e}",
                    self.label,
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

/// Install the SDK's global meter provider when `enabled` — each binary
/// resolves its own `--otel-enabled` / env gate (`PAR_TERM_STREAMER_OTEL_ENABLED`
/// / `PAR_MUX_OTEL_ENABLED`) via clap's `env =` attributes and forwards the
/// values here; `label` (the binary's name) prefixes the stderr diagnostics.
/// Called once from the binary's main/bind, before any instrument is created,
/// so every instrument binds to the real provider. Returns the guard holding
/// the provider — bind it in main for the process lifetime.
pub fn init(
    enabled: bool,
    protocol: OtelProtocol,
    endpoint: &str,
    headers: Vec<(String, String)>,
    service_name: &str,
    label: &'static str,
) -> Option<OtelGuard> {
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
            eprintln!("{label}: OpenTelemetry exporter init failed ({e}); metrics disabled");
            return None;
        }
    };

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name.to_string())
        .build();

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(ReportingExporter {
        inner: exporter,
        label,
        consecutive_failures: std::sync::atomic::AtomicU64::new(0),
    })
    .with_interval(Duration::from_secs(15))
    .build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider.clone());
    Some(OtelGuard { provider, label })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // no-op-waker poll loop stands in for a runtime (no tokio
        // dev-dependency here).
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
                label: "test",
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
                label: "test",
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
}
