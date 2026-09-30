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
# Guarded on ARC-043 (screenshot dep-gating swash) so the script is useful
# before and after that fix lands.
if grep -q 'screenshot = \["dep:swash"\]' Cargo.toml; then
    assert_absent rust-only swash
else
    echo "  skip: rust-only still pulls swash (ARC-043 not landed)"
fi

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

echo ""
echo "All feature checks passed."
