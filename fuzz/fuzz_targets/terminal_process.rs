#![no_main]

//! Whole VTE pipeline: PTY bytes go straight into `Terminal::process`. The
//! read-path calls afterwards (`content`, `capture_snapshot`,
//! `export_scrollback`) exercise the export walkers on the mutated state —
//! a panic or OOB there is as much a finding as one inside the parser.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::terminal::{ExportFormat, Terminal};

fuzz_target!(|data: &[u8]| {
    let mut terminal = Terminal::with_scrollback(80, 24, 1000);
    terminal.process(data);
    let _ = terminal.content();
    let _ = terminal.capture_snapshot();
    let _ = terminal.export_scrollback(ExportFormat::Plain, Some(50));
});
