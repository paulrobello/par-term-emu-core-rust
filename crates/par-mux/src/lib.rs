//! par-mux: the terminal multiplexer daemon library and its attach client.
//!
//! ARC-007 O2 Phase 2: `mux` moved here from the `par-term-emu-core-rust`
//! root crate, which re-exports it as `par_term_emu_core_rust::mux` behind
//! its `mux` feature. See `crates/par-term-emu-core/DESIGN.md`.
#![warn(missing_docs)]

#[cfg(feature = "mux")]
pub mod mux;
