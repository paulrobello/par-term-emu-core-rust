#!/usr/bin/env bash
# Build TerminalCore.xcframework — the C FFI surface (src/ffi.rs,
# include/terminal_core.h) packaged for iOS device + simulator, so an app
# (ParDeck) embeds the same emulator as par-term and the streamer's mirror.
#
# The xcframework builds from the headless profile (rust-only): the default
# `python` feature links the host's libpython, which cannot link into an iOS
# binary (ld: "building for 'iOS', but linking in dylib built for 'macOS'").
#
# Run via `make xcframework`. Requires Xcode (xcodebuild/xcrun/clang) and the
# Rust targets aarch64-apple-ios + aarch64-apple-ios-sim
# (rustup target add aarch64-apple-ios aarch64-apple-ios-sim).
set -euo pipefail

cd "$(dirname "$0")/.."

FEATURES="${XCFRAMEWORK_FEATURES:-rust-only}"
OUT_DIR="${XCFRAMEWORK_OUT_DIR:-target/xcframework}"
DEVICE_TARGET=aarch64-apple-ios
SIM_TARGET=aarch64-apple-ios-sim
LIB_NAME=libpar_term_emu_core_rust.a

if ! command -v xcodebuild >/dev/null 2>&1; then
    echo "ERROR: xcodebuild not found — building the xcframework requires Xcode." >&2
    exit 1
fi

# `cargo rustc --crate-type staticlib` overrides the crate-type for this
# invocation only, so everyday builds (make dev/checkall) never pay the
# extra static-lib link.
for target in "$DEVICE_TARGET" "$SIM_TARGET"; do
    echo "==> Building staticlib: $target (--no-default-features --features $FEATURES)"
    cargo rustc --lib --crate-type staticlib --release \
        --target "$target" --no-default-features --features "$FEATURES"
done

# Smoke-compile the header for both targets: runs its _Static_asserts and
# catches header/API drift before packaging.
echo "==> Header smoke-compile (layout _Static_asserts)"
DEVICE_SDK="$(xcrun --sdk iphoneos --show-sdk-path)"
SIM_SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"
clang -target arm64-apple-ios14.0 -isysroot "$DEVICE_SDK" \
    -fsyntax-only -Wall -Werror include/terminal_core.h
clang -target arm64-apple-ios14.0-simulator -isysroot "$SIM_SDK" \
    -fsyntax-only -Wall -Werror include/terminal_core.h

echo "==> Creating TerminalCore.xcframework"
rm -rf "$OUT_DIR/TerminalCore.xcframework"
mkdir -p "$OUT_DIR"
xcodebuild -create-xcframework \
    -library "target/$DEVICE_TARGET/release/$LIB_NAME" -headers include \
    -library "target/$SIM_TARGET/release/$LIB_NAME" -headers include \
    -output "$OUT_DIR/TerminalCore.xcframework"

# Positive control: exactly two slices with the right platform identity, and
# each archive actually consumable by the Apple linker with the C API
# resolving. (nm is NOT used as a gate: Apple nm's LLVM reader chokes on
# newer rustc bitcode attributes — "Unknown attribute kind (105)" — and
# exits 1 on these archives even though every symbol is present; the link
# probe below is the stronger check anyway.)
echo "==> Verifying slices"
slice_count=$(find "$OUT_DIR/TerminalCore.xcframework" -name "$LIB_NAME" | wc -l | tr -d ' ')
if [ "$slice_count" -ne 2 ]; then
    echo "ERROR: expected 2 library slices in the xcframework, found $slice_count" >&2
    exit 1
fi
# lipo cannot distinguish iOS from simulator (both arm64) — the slice
# directory names xcodebuild emits carry the platform identity.
if [ ! -d "$OUT_DIR/TerminalCore.xcframework/ios-arm64" ]; then
    echo "ERROR: no ios-arm64 (device) slice" >&2
    exit 1
fi
if [ ! -d "$OUT_DIR/TerminalCore.xcframework/ios-arm64-simulator" ] \
    && [ ! -d "$OUT_DIR/TerminalCore.xcframework/ios-arm64_x86_64-simulator" ]; then
    echo "ERROR: no simulator slice" >&2
    exit 1
fi

probe_dir="$(mktemp -d)"
trap 'rm -rf "$probe_dir"' EXIT
cat > "$probe_dir/probe.c" <<'EOF'
#include "terminal_core.h"
int main(void) {
    terminal_free_state(0);
    return 0;
}
EOF
link_probe() { # link_probe <archive> <triple> <sysroot> <min-version-flag>
    local archive="$1" triple="$2" sysroot="$3" minver="$4"
    clang -target "$triple" "$minver" -isysroot "$sysroot" -O0 \
        -Iinclude "$probe_dir/probe.c" "$archive" \
        -o "$probe_dir/probe-$(basename "$(dirname "$archive")")" \
        || return 1
}
for lib in "$OUT_DIR"/TerminalCore.xcframework/*/"$LIB_NAME"; do
    lipo -info "$lib"
    slice="$(basename "$(dirname "$lib")")"
    case "$slice" in
        *simulator*)
            link_probe "$lib" arm64-apple-ios14.0-simulator "$SIM_SDK" "-mios-simulator-version-min=14.0" || {
                echo "ERROR: link probe failed for $slice" >&2
                exit 1
            } ;;
        *)
            link_probe "$lib" arm64-apple-ios14.0 "$DEVICE_SDK" "-mios-version-min=14.0" || {
                echo "ERROR: link probe failed for $slice" >&2
                exit 1
            } ;;
    esac
    echo "    link probe OK: $slice"
done

echo "==> OK: $OUT_DIR/TerminalCore.xcframework (device + simulator)"
