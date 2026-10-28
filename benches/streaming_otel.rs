//! Per-event metrics-recording overhead on the streamer's send path (kanban
//! 01a18331). Run with and without the `otel` feature, interleaved:
//!
//! ```text
//! cargo bench --bench streaming_otel --no-default-features --features rust-only,streaming
//! cargo bench --bench streaming_otel --no-default-features --features rust-only,streaming,otel
//! ```
//!
//! `encode_only` is the baseline in both builds; `encode_plus_record` adds
//! one `Histogram::record` through the in-memory provider (otel build only).

use criterion::{criterion_group, criterion_main, Criterion};
use par_term_emu_core_rust::streaming::protocol::ServerMessage;

fn bench_send_path(c: &mut Criterion) {
    // Without a real provider the instruments are no-ops, which would
    // understate the recording cost; install an in-memory one (otel build).
    #[cfg(feature = "otel")]
    {
        let exporter = opentelemetry_sdk::metrics::InMemoryMetricExporter::default();
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter).build();
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(reader)
            .build();
        opentelemetry::global::set_meter_provider(provider);
    }

    // A 4 KiB Output batch — the streamer batches at 16 ms windows with an
    // 8 KiB cap, so a few KiB is the shape of a busy pane.
    let msg = ServerMessage::output("x".repeat(4096));

    let mut group = c.benchmark_group("streamer_send_path");
    group.throughput(criterion::Throughput::Elements(1));

    group.bench_function("encode_only", |b| {
        b.iter(|| {
            let bytes =
                par_term_emu_core_rust::streaming::proto::encode_server_message(&msg).unwrap();
            std::hint::black_box(bytes)
        })
    });

    #[cfg(feature = "otel")]
    group.bench_function("encode_plus_record", |b| {
        use opentelemetry::metrics::Histogram;
        let hist: Histogram<f64> = opentelemetry::global::meter("bench")
            .f64_histogram("bench.send_latency")
            .build();
        b.iter(|| {
            let encode_started = std::time::Instant::now();
            let bytes =
                par_term_emu_core_rust::streaming::proto::encode_server_message(&msg).unwrap();
            hist.record(encode_started.elapsed().as_secs_f64() * 1000.0, &[]);
            std::hint::black_box(bytes)
        })
    });

    group.finish();
}

criterion_group!(benches, bench_send_path);
criterion_main!(benches);
