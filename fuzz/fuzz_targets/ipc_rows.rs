#![no_main]

//! par-mux reply-row parsers (`ipc::rows`): the attach client reads every
//! `list-agents`/`list-windows`/`list-sessions`/`list-workspaces` and snapshot
//! row from the daemon through these. Invariant: each parser rejects the line
//! or returns a row whose `Display` parses back to itself. Parsed whole from
//! lossy UTF-8, then per line, then with a snapshot-style `<kind> ` prefix
//! stripped (status.rs splits it off before `AgentRow::parse`).

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::mux::ipc::fuzz_support::assert_round_trip;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    assert_round_trip(&text);
    for line in text.split('\n') {
        assert_round_trip(line);
        if let Some((_, rest)) = line.split_once(' ') {
            assert_round_trip(rest);
        }
    }
});
