#!/usr/bin/env bash
# Build TerminalCore.xcframework — the C FFI surface (src/ffi.rs,
# include/terminal_core.h) packaged for iOS device + simulator, so an app
# (ParDeck) embeds the same emulator as par-term and the streamer's mirror.
#
# The xcframework builds from the headless profile (rust-only): the default
# `python` feature links the host's libpython, which cannot link into an iOS
# binary (ld: "building for 'iOS', but linking in dylib built for 'macOS'").
# The `ffi` feature compiles the C surface itself; without it the archive
# exports no ptec_terminal_* symbols (ARC-112).
#
# Run via `make xcframework`. Requires Xcode (xcodebuild/xcrun/clang) and the
# Rust targets aarch64-apple-ios + aarch64-apple-ios-sim
# (rustup target add aarch64-apple-ios aarch64-apple-ios-sim).
set -euo pipefail

cd "$(dirname "$0")/.."

FEATURES="${XCFRAMEWORK_FEATURES:-rust-only,ffi}"
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

# Swift consumers import the module as `import TerminalCore`; without a
# module map in each slice every embedder must hand-roll a bridging header
# (verified 2026-09-27 from ParDeck). -create-xcframework has no flag for
# this, so write it into every slice post-create.
echo "==> Writing Modules/module.modulemap into slices"
for slice_dir in "$OUT_DIR"/TerminalCore.xcframework/ios-*; do
    [ -d "$slice_dir" ] || continue
    mkdir -p "$slice_dir/Modules"
    cat > "$slice_dir/Modules/module.modulemap" <<'MAP'
module TerminalCore {
    header "../Headers/terminal_core.h"
    export *
}
MAP
done

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
# The probe names v4-only surface (the ptec_ prefix, the grapheme side
# channel, TermEvent) so a stale archive or header fails the link.
cat > "$probe_dir/probe.c" <<'EOF'
#include "terminal_core.h"
int main(void) {
    ptec_terminal_free_state(0);
    TermEvent ev = { .kind = TERM_EVENT_TITLE_CHANGED };
    (void)ev;
    return (int)ptec_terminal_read_cell_grapheme(0, 0, 0, 0, 0)
        + (ptec_terminal_abi_version() == TERM_CORE_ABI_VERSION ? 0 : 1);
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

# Swift import probe: the module map must make the C API importable from
# Swift (typecheck only — linking is already covered by the probes above).
sim_slice="$(find "$OUT_DIR/TerminalCore.xcframework" -maxdepth 1 -type d -name '*simulator*' | head -1)"
cat > "$probe_dir/probe.swift" <<'EOF'
import TerminalCore

let ev = TermKeyEvent(key: UInt16(TERM_KEY_ENTER), modifiers: 0, _pad: 0, codepoint: 0)
ptec_terminal_free_state(nil)
_ = ev
let combining = UInt16(TERM_ATTR_HAS_COMBINING)
let kind = UInt16(TERM_EVENT_TITLE_CHANGED)
_ = ptec_terminal_read_cell_grapheme(nil, 0, 0, nil, 0)
var vtable = TerminalObserverVtable()
vtable.on_event_v2 = { _, event in _ = event?.pointee.payload_len }
_ = (combining, kind, vtable)
EOF
if [ ! -f "$sim_slice/Modules/module.modulemap" ]; then
    echo "ERROR: no Modules/module.modulemap in $sim_slice" >&2
    exit 1
fi
xcrun -sdk iphonesimulator swiftc -target arm64-apple-ios14.0-simulator \
    -typecheck -Xcc -fmodule-map-file="$sim_slice/Modules/module.modulemap" \
    "$probe_dir/probe.swift" || {
        echo "ERROR: Swift import probe failed" >&2
        exit 1
    }
echo "    Swift import probe OK: import TerminalCore typechecks"

echo "==> OK: $OUT_DIR/TerminalCore.xcframework (device + simulator)"
