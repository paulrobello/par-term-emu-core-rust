//! Guards the feature isolation D1 depends on.
//!
//! `sim` is a PTY-free, runtime-free profile for embedders that vendor the
//! crate as a pure screen model. `mux` pulls in `portable-pty` (real PTYs
//! and a local socket), so it must never reach that profile — or the
//! default one.

/// The `mux` module must exist when the feature is on.
#[cfg(feature = "mux")]
#[test]
fn mux_module_is_reachable_with_the_feature() {
    // Referencing the module is the assertion; this fails to compile if the
    // module is not declared or not gated correctly.
    let _ = std::any::type_name::<par_term_emu_core_rust::mux::MuxMarker>();
}

/// `mux` must be absent from any build that did not ask for it.
#[cfg(not(feature = "mux"))]
#[test]
fn mux_is_absent_without_the_feature() {
    // Nothing to call — the guarantee is that the crate compiled at all in a
    // profile where `mux`'s dependencies are not present. The companion CI
    // command is the real enforcement:
    //   cargo test --lib --no-default-features
    //   cargo build --no-default-features --features sim
}
