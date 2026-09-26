#![no_main]

//! mux hook-report JSON grammar: the `{`-shaped half of the control socket.
//! `handle_report` byte-sniffs nothing itself — it takes the line `parse_line`
//! classified as [`Line::Hook`] and walks JSON parse, header validation, the
//! SEC-105 value-length caps, and (for known panes) the pane-metadata write.
//! The tree is empty, so accepted-grammar reports stop at the pane lookup —
//! exactly the boundary: everything a malformed report can corrupt before a
//! real pane exists. Invariants: no panic, and a reply always comes back.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::mux::hooks::handle_report;
use par_term_emu_core_rust::mux::pane::ShellPaneFactory;
use par_term_emu_core_rust::mux::tree::MuxTree;
use parking_lot::Mutex;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let tree = Arc::new(Mutex::new(MuxTree::new(Box::new(
        ShellPaneFactory::default(),
    ))));
    let _ = handle_report(&text, &tree);
});
