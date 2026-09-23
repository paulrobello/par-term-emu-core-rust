#![no_main]

//! tmux control-mode parser: `%output`/`%pane` notification lines carved out
//! of a PTY stream. Parsed once whole, then re-parsed split at every byte
//! offset in a 64-byte window — the parser's partial-line buffering is the
//! untested half of its surface (a real daemon delivers arbitrary chunk
//! boundaries).

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::tmux_control::TmuxControlParser;

fuzz_target!(|data: &[u8]| {
    let mut parser = TmuxControlParser::new(true);
    let _ = parser.parse(data);

    for split in 0..data.len().min(64) {
        let mut parser = TmuxControlParser::new(true);
        let _ = parser.parse(&data[..split]);
        let _ = parser.parse(&data[split..]);
    }
});
