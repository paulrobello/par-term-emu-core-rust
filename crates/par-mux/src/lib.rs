//! par-mux: the terminal multiplexer daemon library and its attach client.
//!
//! ARC-007 O2 Phase 2: `mux` moved here from the `par-term-emu-core-rust`
//! root crate, which re-exports it as `par_term_emu_core_rust::mux` behind
//! its `mux` feature. See `crates/par-term-emu-core/DESIGN.md`.
#![warn(missing_docs)]

// The moved tree addresses the terminal core through `crate::<module>` (its
// paths from before the split); these root-level re-exports keep every one
// of them resolving unchanged.
#[cfg(all(feature = "mux", feature = "attach"))]
pub(crate) use par_term_emu_core::cursor;
#[cfg(all(feature = "mux", any(test, feature = "attach")))]
pub(crate) use par_term_emu_core::mouse;
#[cfg(all(feature = "mux", test))]
pub(crate) use par_term_emu_core::zone;
#[cfg(feature = "mux")]
pub(crate) use par_term_emu_core::{
    cell, color, debug_error, debug_log, keyboard, pty_error, pty_session, terminal, tmux_control,
};

#[cfg(feature = "mux")]
pub mod mux;
