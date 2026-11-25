//! SEC-010: Terminal::transfer_summaries must allocate nothing
//! proportional to payload size. This lives in its own test binary so its
//! counting global allocator measures the accessor's allocation delta
//! directly, with no other tests polluting the count.
//!
//! Two terminals each hold one ACTIVE transfer with identical metadata
//! ("same.bin", same direction) but payloads differing by 50 MiB; the
//! allocation delta of one transfer_summaries() poll must be
//! byte-identical between them.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use par_term_emu_core::terminal::Terminal;
use parking_lot::RwLock;

struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocated() -> usize {
    ALLOCATED.load(Ordering::Relaxed)
}

/// Feed one MultipartFile transfer named `name` with `size` declared
/// bytes, appending `chunk` repeatedly until `payload_bytes` have been
/// fed. The transfer stays ACTIVE because the declared size leaves
/// headroom over the payload.
fn feed_transfer(term: &mut Terminal, name: &str, size: usize, payload_bytes: usize, chunk: &[u8]) {
    let header = format!(
        "\x1b]1337;MultipartFile=size={size};name={}\x07",
        B64.encode(name.as_bytes())
    );
    term.process(header.as_bytes());
    let encoded = B64.encode(chunk);
    let part = format!("\x1b]1337;FilePart={encoded}\x07");
    let mut fed = 0;
    while fed < payload_bytes {
        term.process(part.as_bytes());
        fed += chunk.len();
    }
}

fn poll_delta(term: &Arc<RwLock<Terminal>>) -> usize {
    let before = allocated();
    let _summaries = term.read().transfer_summaries();
    allocated() - before
}

/// Append `payload_bytes` more via FilePart to an in-flight multipart
/// transfer, completing it once its declared size is reached.
fn feed_more(term: &mut Terminal, chunk: &[u8], payload_bytes: usize) {
    let encoded = B64.encode(chunk);
    let part = format!("\x1b]1337;FilePart={encoded}\x07");
    let mut fed = 0;
    while fed < payload_bytes {
        term.process(part.as_bytes());
        fed += chunk.len();
    }
}

fn unwrap_term(term: Arc<RwLock<Terminal>>) -> Terminal {
    Arc::try_unwrap(term)
        .expect("the test holds the sole Arc")
        .into_inner()
}

#[test]
fn a_transfer_summary_poll_allocates_nothing_payload_proportional() {
    // Feed 50 MiB in parser-sized chunks, versus a single 1-byte chunk.
    // The declared size leaves headroom so the transfer stays ACTIVE.
    let big = vec![b'x'; 4 * 1024];
    let tiny = vec![b'z'; 1];

    let mut big_term = Terminal::new(80, 24);
    big_term.set_max_transfer_size(64 * 1024 * 1024);
    feed_transfer(
        &mut big_term,
        "same.bin",
        52 * 1024 * 1024,
        50 * 1024 * 1024,
        &big,
    );
    let big_term = Arc::new(RwLock::new(big_term));
    assert_eq!(big_term.read().transfer_summaries().active.len(), 1);
    assert_eq!(
        big_term.read().transfer_summaries().active[0].bytes_transferred,
        50 * 1024 * 1024,
        "the 50 MiB payload must be live in the terminal during the poll"
    );

    let mut small_term = Terminal::new(80, 24);
    small_term.set_max_transfer_size(64 * 1024 * 1024);
    feed_transfer(&mut small_term, "same.bin", 16, 1, &tiny);
    let small_term = Arc::new(RwLock::new(small_term));
    assert_eq!(small_term.read().transfer_summaries().active.len(), 1);

    let big_delta = poll_delta(&big_term);
    let small_delta = poll_delta(&small_term);

    assert_eq!(
        big_delta, small_delta,
        "polling a 50 MiB transfer must allocate exactly what polling a 1-byte one does"
    );
    assert!(
        big_delta < 4096,
        "a one-transfer poll is small metadata: {big_delta} bytes allocated"
    );

    // The completed ring gets the same proof: complete both transfers at
    // their declared sizes and repeat the comparison. The 50 MiB payload
    // is retained in the ring (auto-completion keeps the bytes), so any
    // payload-proportional allocation in the completed path would show.
    let mut big_term = unwrap_term(big_term);
    feed_more(&mut big_term, &big, 2 * 1024 * 1024);
    let big_term = Arc::new(RwLock::new(big_term));
    assert_eq!(big_term.read().transfer_summaries().active.len(), 0);
    assert_eq!(big_term.read().transfer_summaries().completed.len(), 1);
    assert_eq!(
        big_term.read().transfer_summaries().completed[0].bytes_transferred,
        52 * 1024 * 1024,
        "the full payload must be retained in the completed ring"
    );

    let mut small_term = unwrap_term(small_term);
    feed_more(&mut small_term, &tiny, 15);
    let small_term = Arc::new(RwLock::new(small_term));
    assert_eq!(small_term.read().transfer_summaries().completed.len(), 1);

    let big_completed_delta = poll_delta(&big_term);
    let small_completed_delta = poll_delta(&small_term);
    assert_eq!(
        big_completed_delta, small_completed_delta,
        "polling the completed ring must allocate the same for a 52 MiB retained payload as for a 16-byte one"
    );
    assert!(
        big_completed_delta < 4096,
        "a completed-ring poll is small metadata: {big_completed_delta} bytes allocated"
    );
}
