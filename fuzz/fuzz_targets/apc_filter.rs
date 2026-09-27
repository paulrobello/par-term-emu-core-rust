#![no_main]

//! APC pre-filter byte state machine (ENH-020). Every terminal byte walks
//! this state machine before `vte` sees it, so a panic or an unbounded
//! accumulation here hangs or bloats every consumer. Drives the filter
//! directly (one `feed` pass from a fresh state) rather than through
//! `Terminal::process`, which the `terminal_process` target already covers.
//! Memory bound: the accumulator is unbounded pending SEC-109 — runs are
//! expected to stay under `-rss_limit_mb=512` on non-bomb inputs, and a
//! crafted bomb input is exactly what that limit is there to catch.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::terminal::fuzz_apc_filter;

fuzz_target!(|data: &[u8]| {
    let _ = fuzz_apc_filter(data);
});
