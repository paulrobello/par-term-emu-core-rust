#![no_main]

//! Kitty graphics APC parser. Input is decoded lossily and split on ST
//! (`ESC \`), so each segment is one APC; a leading `ESC _ G` intro is
//! stripped the way the terminal's APC filter does before handing the
//! payload to `parse_chunk`. Return-value semantics mirror the real caller
//! (src/terminal/mod.rs `filter_apc_and_advance`): `Ok(true)` is an m=1
//! continuation — keep parser state — and `Ok(false)` is the final chunk,
//! which builds the graphic into a fresh `GraphicsStore` and resets.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::graphics::kitty::KittyParser;
use par_term_emu_core_rust::graphics::GraphicsStore;

fuzz_target!(|data: &[u8]| {
    let payload = String::from_utf8_lossy(data);
    let mut parser = KittyParser::new();
    let mut store = GraphicsStore::new();
    for chunk in payload.split("\x1b\\") {
        let chunk = chunk
            .strip_prefix("\x1b_G")
            .or_else(|| chunk.strip_prefix("_G"))
            .unwrap_or(chunk);
        if chunk.is_empty() {
            continue;
        }
        match parser.parse_chunk(chunk) {
            Ok(true) => {}
            Ok(false) => {
                let _ = parser.build_graphic((0, 0), &mut store);
                parser = KittyParser::new();
            }
            Err(_) => parser = KittyParser::new(),
        }
    }
});
