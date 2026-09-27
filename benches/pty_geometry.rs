//! PtySession geometry getters under reader-thread contention (ENH-023).
//!
//! A background feeder holds the terminal write lock in bursts — one 1 MiB
//! `process()` per iteration, the shape of a PTY output burst — while the
//! benchmark thread polls `size()`: once through the wait-free atomic
//! mirror, once through the QA-130 read-lock baseline it replaces. The two
//! variants alternate within one process (the interleaved A/B discipline:
//! machine-state swings between sequential runs make single-binary runs
//! incomparable), and the whole binary should be run a few times when a
//! decision hangs on the ratio.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use par_term_emu_core_rust::pty_session::PtySession;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The reader-thread write-lock pattern: process a 1 MiB chunk per
/// iteration until told to stop, reporting how many chunks landed (a
/// sanity floor for "the lock was actually contended during the run").
fn spawn_feeder(session: &PtySession, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<usize> {
    let terminal = session.terminal_ref().clone();
    std::thread::spawn(move || {
        let chunk = vec![b'x'; 1024 * 1024];
        let mut fed = 0usize;
        while !stop.load(Ordering::Relaxed) {
            let mut term = terminal.write();
            term.process(&chunk);
            fed += 1;
        }
        fed
    })
}

fn bench_geometry(c: &mut Criterion) {
    let session = PtySession::new(80, 24, 1000);
    let stop = Arc::new(AtomicBool::new(false));
    let feeder = spawn_feeder(&session, stop.clone());
    // Let the feeder's first write lock land before measuring, so even the
    // first samples see a contended lock.
    std::thread::sleep(Duration::from_millis(100));

    let mut group = c.benchmark_group("pty_geometry_size");
    group.throughput(Throughput::Elements(1));
    // Two interleaved rounds: order effects (thermal/machine state) hit
    // both variants in both positions.
    for round in 0..2 {
        group.bench_with_input(
            BenchmarkId::new("size_atomic_mirror", format!("round{round}")),
            &round,
            |b, _| b.iter(|| session.size()),
        );
        group.bench_with_input(
            BenchmarkId::new("size_read_lock_baseline", format!("round{round}")),
            &round,
            |b, _| {
                b.iter(|| {
                    let term = session.terminal_ref().read();
                    term.size()
                })
            },
        );
    }
    group.finish();

    stop.store(true, Ordering::Relaxed);
    let fed = feeder.join().expect("feeder joins");
    eprintln!("feeder processed {fed} MiB chunks during the bench");
}

criterion_group!(benches, bench_geometry);
criterion_main!(benches);
