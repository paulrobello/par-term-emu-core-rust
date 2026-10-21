#!/bin/sh
# par-mux installer — the curl|sh path:
#   curl -fsSL https://raw.githubusercontent.com/paulrobello/par-term-emu-core-rust/main/scripts/install-par-mux.sh | sh
# Downloads the newest release archive for this platform, verifies its
# SHA256 against the release's SHA256SUMS, and installs the par-mux binary
# into ~/.local/bin (override: PAR_MUX_INSTALL_DIR). No root, no daemon
# start — run `par-mux` yourself after.
set -eu

REPO="paulrobello/par-term-emu-core-rust"
INSTALL_DIR="${PAR_MUX_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) TARGET="aarch64-apple-darwin" ;;
    Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
    Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
    Linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
    *) echo "unsupported platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "tar is required" >&2; exit 1; }

echo "resolving the latest release"
VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
[ -n "$VERSION" ] || { echo "could not resolve the latest release" >&2; exit 1; }

ARCHIVE="par-mux-$VERSION-$TARGET.tar.gz"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "downloading $ARCHIVE"
curl -fsSL -o "$WORK/$ARCHIVE" "https://github.com/$REPO/releases/download/$VERSION/$ARCHIVE"
curl -fsSL -o "$WORK/SHA256SUMS" "https://github.com/$REPO/releases/download/$VERSION/par-mux-$VERSION-SHA256SUMS.txt"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$WORK" && grep " $ARCHIVE\$" SHA256SUMS | sha256sum -c -) \
        || { echo "checksum mismatch" >&2; exit 1; }
else
    echo "sha256sum not found; skipping checksum verification" >&2
fi

tar -xzf "$WORK/$ARCHIVE" -C "$WORK"
mkdir -p "$INSTALL_DIR"
mv "$WORK/par-mux" "$INSTALL_DIR/par-mux"
chmod +x "$INSTALL_DIR/par-mux"

echo "installed $("$INSTALL_DIR/par-mux" --version | head -1)"
echo "  binary: $INSTALL_DIR/par-mux"
echo "  make sure $INSTALL_DIR is on your PATH; completions: par-mux completion <shell>"
