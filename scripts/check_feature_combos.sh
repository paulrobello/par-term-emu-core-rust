#!/usr/bin/env bash
# ARC-005: compile-check the feature combinations real consumers build, plus
# the pairings no named profile covers.
#
# `make check-features` proves each feature alone and the dependency-tree
# isolation of the slim profiles. It does not prove that a cfg combination
# compiles when two features meet. A combination that breaks ships to an
# embedder before anyone in-repo builds it, so this gate pins the list:
# bounded, deterministic, run by `make checkall-ci` (not `checkall`, which
# stays fast). A powerset (`cargo hack --feature-powerset --depth 2`) over
# ~20 forwarding features would add minutes of cold builds for combinations
# nobody ships; the pinned list names the ones someone does.
#
# One `cargo check` per line, warm after the first. Add a combination here
# when a consumer starts shipping it or a cfg bug is found in one.
#
# `streaming` does NOT imply `pty_session` (only `streaming-bin`, `python`,
# and `mux` do), so `streaming` alone is a real PTY-free embedder profile.
# It is checked bare and with the PTY backend because the session-factory
# seams differ.
#
# `otel` on Windows is deliberately absent. `otel` implies `streaming` at the
# root, and the members' `otel` pulls opentelemetry-otlp -> rustls ->
# aws-lc-sys, whose C build needs MSVC headers a macOS cross build lacks (the
# same reason `cross-target-check` excludes `streaming`). CI's windows-latest
# job and the local Parallels Windows VM (CLAUDE.md, "Local Windows VM") cover
# it natively.
set -euo pipefail

cd "$(dirname "$0")/.."

run() {
    echo "--- cargo check $*"
    cargo check --locked "$@"
}

echo "=== 1/2 consumer profiles ==="
# par-term (Cargo.toml): default-features = false, rust-only + pty_session +
# screenshot; its mux builds add `mux`.
run --no-default-features --features rust-only,pty_session,screenshot --lib
run --no-default-features --features rust-only,pty_session,screenshot,mux --lib
# The headless `sim` profile (par-hack's server-side screen model), with and
# without the opt-in renderer (ENH-024).
run --no-default-features --features sim --lib
run --no-default-features --features sim,screenshot --lib

echo "=== 2/2 pairings no named profile covers ==="
run --no-default-features --features rust-only,streaming --lib
run --no-default-features --features rust-only,streaming,pty_session --lib
run --no-default-features --features rust-only,ffi,mux --lib
run --no-default-features --features rust-only,ffi,mux-bin,attach --lib
run --no-default-features --features rust-only,otel,mux --lib
run --no-default-features --features rust-only,streaming,mux,serde --lib
run -p par-mux --features mux-bin,otel --lib
run -p par-term-emu-core --features otel,pty_session,screenshot --lib

echo ""
echo "All feature-combination checks passed."
