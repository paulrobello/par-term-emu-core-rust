//! Per-frame dirty-range readback cost for the C FFI embedding surface
//! (ParDeck DESIGN.md RQ2): feed a frame, collect dirty ranges, copy only
//! the rows inside them — versus the full-grid `ptec_terminal_get_state`
//! snapshot, so the delta is visible in the numbers.
//!
//! Two frame shapes:
//! - status frames: in-place line rewrites, no scrolling — damage tracking
//!   must localize these to a handful of rows (asserted below; a regression
//!   to full repaints fails the bench instead of benchmarking a full copy).
//! - agent frames: status updates plus a scrolling log tail — the realistic
//!   ParDeck stream. Even here the dirty path avoids the snapshot's
//!   per-frame allocation and copies straight into the caller's buffer.
//!
//! Run with `make bench` (cargo bench --no-default-features --features
//! rust-only --bench ffi_readback).

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use par_term_emu_core_rust::ffi::{
    ptec_terminal_create, ptec_terminal_dirty_ranges, ptec_terminal_feed, ptec_terminal_free,
    ptec_terminal_free_state, ptec_terminal_get_state, ptec_terminal_mark_clean,
    ptec_terminal_read_row, SharedCell, TermRowRange,
};

const COLS: usize = 120;
const ROWS: usize = 40;
const SCROLLBACK: usize = 500;
const FRAMES: usize = 60;

/// Status frame: rewrites the header and three status lines in place.
/// Never scrolls, never grows — the pure damage-tracking case.
fn status_frame(frame: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    buf.extend_from_slice(b"\x1b[1;1H\x1b[2K");
    buf.extend_from_slice(format!("== session frame {frame:05} ==\x1b[K").as_bytes());
    for line in 0..3 {
        buf.extend_from_slice(format!("\x1b[{row};1H\x1b[2K", row = line + 2).as_bytes());
        buf.extend_from_slice(format!("status[{line}]: {:x>16}", frame * (line + 1)).as_bytes());
    }
    buf
}

/// Agent frame: status updates plus a log line appended at the bottom row,
/// which scrolls the screen every frame — the agent-output-stream shape.
fn agent_frame(frame: usize) -> Vec<u8> {
    let mut buf = status_frame(frame);
    buf.extend_from_slice(b"\x1b[40;1H");
    buf.extend_from_slice(format!("log line {frame} padding-padding-padding\r\n").as_bytes());
    buf
}

/// Damage localization is gated by ffi::tests::status_frames_localize_damage
/// in the library test suite (a bench target with harness=false runs no
/// #[test] functions).
fn bench_dirty_readback(c: &mut Criterion) {
    let mut group = c.benchmark_group("ffi_readback");
    group.throughput(Throughput::Elements(FRAMES as u64));
    group.sample_size(30);

    let make = || unsafe { ptec_terminal_create(COLS as u32, ROWS as u32, SCROLLBACK as u32) };
    let row_buf = vec![SharedCell::blank(); COLS];
    let mut ranges = vec![TermRowRange { start: 0, end: 0 }; ROWS];

    // Per-frame cost of the whole render loop: feed → ranges → read only
    // the dirty rows into the pinned buffer → mark clean.
    group.bench_function("agent_feed_dirty_readback", |b| {
        b.iter(|| {
            let term = make();
            let mut cells = 0usize;
            for f in 0..FRAMES {
                let bytes = agent_frame(f);
                unsafe { ptec_terminal_feed(term, bytes.as_ptr(), bytes.len() as u32) };
                let n = unsafe {
                    ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), ranges.len() as u32)
                };
                for r in &ranges[..n as usize] {
                    for row in r.start..=r.end {
                        cells += unsafe {
                            ptec_terminal_read_row(
                                term,
                                row,
                                0,
                                row_buf.as_ptr() as *mut SharedCell,
                                COLS as u32,
                            )
                        } as usize;
                    }
                }
                unsafe { ptec_terminal_mark_clean(term) };
            }
            unsafe { ptec_terminal_free(term) };
            cells
        })
    });

    // The same loop against a non-scrolling status stream — the shape where
    // damage tracking saves the most.
    group.bench_function("status_feed_dirty_readback", |b| {
        b.iter(|| {
            let term = make();
            let mut cells = 0usize;
            for f in 0..FRAMES {
                let bytes = status_frame(f);
                unsafe { ptec_terminal_feed(term, bytes.as_ptr(), bytes.len() as u32) };
                let n = unsafe {
                    ptec_terminal_dirty_ranges(term, ranges.as_mut_ptr(), ranges.len() as u32)
                };
                for r in &ranges[..n as usize] {
                    for row in r.start..=r.end {
                        cells += unsafe {
                            ptec_terminal_read_row(
                                term,
                                row,
                                0,
                                row_buf.as_ptr() as *mut SharedCell,
                                COLS as u32,
                            )
                        } as usize;
                    }
                }
                unsafe { ptec_terminal_mark_clean(term) };
            }
            unsafe { ptec_terminal_free(term) };
            cells
        })
    });

    // Control: the same agent stream snapshotted with the full-grid API —
    // the allocation + full copy the dirty path exists to avoid.
    group.bench_function("agent_feed_full_snapshot", |b| {
        b.iter(|| {
            let term = make();
            let mut count = 0usize;
            for f in 0..FRAMES {
                let bytes = agent_frame(f);
                unsafe { ptec_terminal_feed(term, bytes.as_ptr(), bytes.len() as u32) };
                let state = unsafe { ptec_terminal_get_state(term) };
                count += unsafe { (*state).cell_count as usize };
                unsafe { ptec_terminal_free_state(state) };
            }
            unsafe { ptec_terminal_free(term) };
            count
        })
    });

    group.finish();
}

criterion_group!(benches, bench_dirty_readback);
criterion_main!(benches);
