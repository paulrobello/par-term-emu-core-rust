// ENH-026: the FFI dirty-range calls must not allocate — the render loop
// calls them twice per frame. This binary holds exactly one test so the
// counting allocator's window contains only this test's FFI calls; a
// sibling test allocating concurrently would make the zero flaky.
use par_term_emu_core_rust::ffi::{ptec_terminal_dirty_ranges, TermRowRange};
use par_term_emu_core_rust::terminal::Terminal;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

#[test]
fn terminal_dirty_ranges_does_not_allocate() {
    // Dirtied through the public surface only: two singles, a mid-screen
    // run, and a run crossing the 64-row boundary. The caller buffer is
    // pre-allocated outside the measured window.
    let mut term = Terminal::with_scrollback(80, 200, 0);
    term.process(b"\x1b[3;1HX\x1b[5;1HX\x1b[10;1HX\x1b[11;1HX\x1b[12;1HX\x1b[63;1HX\x1b[64;1HX\x1b[65;1HX\x1b[66;1HX\x1b[67;1HX");

    let mut buf = vec![TermRowRange { start: 0, end: 0 }; 64];

    let before = ALLOCS.load(Relaxed);
    let fill = unsafe { ptec_terminal_dirty_ranges(&term, buf.as_mut_ptr(), 64) };
    let sizing = unsafe { ptec_terminal_dirty_ranges(&term, std::ptr::null_mut(), 0) };
    let after = ALLOCS.load(Relaxed);

    assert_eq!(
        after - before,
        0,
        "dirty-range calls allocated; buf holds {fill} of {sizing} ranges"
    );
    assert_eq!(fill, sizing);
    assert_eq!(
        &buf[..fill as usize],
        &[
            TermRowRange { start: 2, end: 2 },
            TermRowRange { start: 4, end: 4 },
            TermRowRange { start: 9, end: 11 },
            TermRowRange { start: 62, end: 66 },
        ],
        "coalesced runs through the C ABI"
    );
}
