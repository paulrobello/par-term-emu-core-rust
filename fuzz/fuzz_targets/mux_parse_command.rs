#![no_main]

//! mux control-protocol command parser: `parse_line`/`parse_command` over
//! arbitrary client lines. The daemon executes what this parser returns
//! (spawns panes, mutates the session tree), so a panic or a mis-parse here
//! is pre-dispatch corruption. Parsed once whole from lossy-UTF-8, then as
//! every line of a multi-line input — the wire protocol is line-based and
//! `%`-notification-looking and `{`-hook-looking lines ride the same socket.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::mux::command::{parse_command, parse_line};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = parse_command(&text);
    let _ = parse_line(&text);

    for line in text.split('\n') {
        let _ = parse_command(line);
        let _ = parse_line(line);
    }
});
