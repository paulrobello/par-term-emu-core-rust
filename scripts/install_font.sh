#!/bin/bash
# Download a free monospace font for screenshot support
# This script downloads Hack font which is open source (MIT/Bitstream Vera license)

set -e

FONT_DIR="${HOME}/.local/share/fonts"
FONT_NAME="Hack-Regular.ttf"
FONT_URL="https://github.com/source-foundry/Hack/releases/download/v3.003/Hack-v3.003-ttf.zip"
TEMP_DIR=$(mktemp -d)

echo "📥 Downloading Hack font for screenshot support..."
echo "   This font is licensed under MIT/Bitstream Vera License"
echo

# Create font directory if it doesn't exist
mkdir -p "$FONT_DIR"

# Download and extract
cd "$TEMP_DIR"
curl -L -o hack.zip "$FONT_URL"
unzip -q hack.zip

# Copy the regular font
cp "ttf/Hack-Regular.ttf" "$FONT_DIR/"

# Update font cache on Linux
if command -v fc-cache &> /dev/null; then
    echo "🔄 Updating font cache..."
    fc-cache -f "$FONT_DIR"
fi

# Clean up
rm -rf "$TEMP_DIR"

echo "✅ Font installed successfully to: $FONT_DIR/Hack-Regular.ttf"
echo
echo "You can now use screenshots without specifying a font path:"
echo "  term.screenshot()"
echo
echo "Or specify the font explicitly:"
echo "  term.screenshot(font_path=\"$FONT_DIR/Hack-Regular.ttf\")"
