#![no_main]

//! Sixel state machine. Mirrors the byte dispatch `dcs_put`
//! (src/terminal/sequences/dcs/mod.rs) applies to sixel data: bytes 63..=126
//! are pixel data, `$` is carriage return, `-` is newline. The `#`/`"`/`!`
//! command forms above it are covered by the terminal_process target. The
//! asserts are the proof that `SixelLimits` holds against mutated input.

use libfuzzer_sys::fuzz_target;
use par_term_emu_core_rust::sixel::{SixelLimits, SixelParser};

fuzz_target!(|data: &[u8]| {
    let limits = SixelLimits::new(4096, 4096, 65535);
    let mut parser = SixelParser::new_with_limits(limits);
    parser.set_params(&[1, 1]);
    for &byte in data {
        match byte {
            63..=126 => parser.parse_sixel(byte as char),
            b'$' => parser.carriage_return(),
            b'-' => parser.new_line(),
            _ => {}
        }
    }
    let graphic = parser.build_graphic((0, 0));
    assert!(graphic.width <= limits.max_width);
    assert!(graphic.height <= limits.max_height);
});
