#!/usr/bin/env bash
# Feature-hygiene gate (ENH-019): every feature builds on its own, and heavy
# dependencies stay out of the slim profiles. Catches the ARC-043 class
# (`screenshot = []` leaving swash unconditional) and the ARC-044 class (`mux`
# pulling binary-only clap; binary split out as `mux-bin` by ARC-106)
# mechanically, before they ship.
#
# Run via `make check-features` after any [features] or dependency edit in
# Cargo.toml. Not part of `make checkall` — the matrix takes minutes.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-hack >/dev/null 2>&1; then
    echo "ERROR: cargo-hack is not installed." >&2
    echo "  cargo install cargo-hack --locked" >&2
    exit 1
fi

echo "=== 1/3 cargo hack: every feature checks on its own ==="
# python/python-test need a Python interpreter to build (checked separately in
# step 2); full is a superset of python; regenerate-proto needs protoc;
# jemalloc does not exist on Windows (the CI job runs this on Linux, but the
# script must stay usable from macOS/Windows checkouts too).
# --each-feature runs each feature on its own with default features off.
cargo hack check --each-feature \
    --exclude-features python,python-test,full,regenerate-proto,jemalloc
# The par-term-emu-core workspace member (ARC-007 O2): each of its features
# on its own, plus none. Its `python` feature only adds pyo3 for the PyErr
# conversions (pyo3's build script still locates an interpreter, as for
# python-test below).
cargo hack check -p par-term-emu-core --each-feature
# The par-mux member (ARC-007 O2 Phase 2): none, each feature alone, and all.
cargo hack check -p par-mux --each-feature
cargo check -p par-mux --all-features --all-targets

echo "=== 2/3 explicit combinations + python-test ==="
cargo check --no-default-features --features rust-only,mux
cargo check --no-default-features --features rust-only,mux-bin
cargo check --no-default-features --features rust-only,streaming
cargo check --no-default-features --features streaming-bin
# The C ABI surface the xcframework builds (ARC-112).
cargo check --no-default-features --features rust-only,ffi
# pyo3 needs an interpreter at build-script time; the lib alone is enough to
# prove the bindings' feature surface still compiles.
cargo check --no-default-features --features python-test --lib

echo "=== 3/3 dependency-tree assertions ==="
# assert_absent <features> <crate>: fail if <crate> resolves into the normal
# dependency tree of that feature set. `cargo tree -i` exits 0 whether or not
# the crate resolves, so presence is "stdout is non-empty" — an absent crate
# prints only a stderr "nothing to print" warning.
assert_absent() {
    local features="$1" crate="$2"
    if cargo tree --no-default-features --features "$features" -e normal -i "$crate" 2>/dev/null | grep -q .; then
        echo "FAIL: '$crate' is in the dependency tree of --features $features" >&2
        exit 1
    fi
    echo "  ok: $crate absent from --features $features"
}

# rust-only: no Python, no async runtime, no CLI parser.
assert_absent rust-only pyo3
assert_absent rust-only tokio
assert_absent rust-only clap
# Macro YAML is the `macro-yaml` feature's alone (ARC-116).
assert_absent rust-only serde_yaml_ng
# ARC-043: swash is the `screenshot` feature's alone (gated in the member
# since ARC-007 O2; the root `screenshot` forwards to it).
assert_absent rust-only swash

# sim: headless profile — no PTY backend, no Python, no async runtime, and no
# font-rendering stack (ENH-024: sim no longer implies screenshot).
assert_absent sim portable-pty
assert_absent sim pyo3
assert_absent sim tokio
assert_absent sim swash
assert_absent sim serde_yaml_ng

# mux: a daemon, not a WebSocket client — no streaming client stack.
assert_absent rust-only,mux tokio-tungstenite
# The CLI parser belongs to the `mux-bin` binary feature only (ARC-106).
assert_absent rust-only,mux clap

# streaming (the library feature): the CLI/logging/download deps belong to
# streaming-bin only.
assert_absent rust-only,streaming clap
assert_absent rust-only,streaming tracing-subscriber

# par-term-emu-core with no features is the headless profile: no Python, no
# PTY backend, no font stack, no YAML.
assert_member_absent() {
    local crate="$1"
    if cargo tree -p par-term-emu-core --no-default-features -e normal -i "$crate" 2>/dev/null | grep -q .; then
        echo "FAIL: '$crate' is in the dependency tree of par-term-emu-core (no features)" >&2
        exit 1
    fi
    echo "  ok: $crate absent from par-term-emu-core (no features)"
}
assert_member_absent pyo3
assert_member_absent portable-pty
assert_member_absent swash
assert_member_absent serde_yaml_ng
assert_member_absent tokio

# par-mux: the `mux` library never pulls the binary-only CLI parser, the
# attach TUI stack, or the streaming runtime (ARC-106 / D1).
assert_mux_absent() {
    local features="$1" crate="$2"
    if cargo tree -p par-mux --features "$features" -e normal -i "$crate" 2>/dev/null | grep -q .; then
        echo "FAIL: '$crate' is in the dependency tree of par-mux --features $features" >&2
        exit 1
    fi
    echo "  ok: $crate absent from par-mux --features $features"
}
assert_mux_absent mux clap
assert_mux_absent mux crossterm
assert_mux_absent mux ratatui
assert_mux_absent mux tokio
assert_mux_absent mux pyo3

echo ""
echo "All feature checks passed."
