#!/usr/bin/env bash
# Install script for par-term terminfo definition
# Usage: ./install.sh [--system]

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TERMINFO_FILE="${SCRIPT_DIR}/par-term.ti"

# Check if terminfo file exists
if [[ ! -f "${TERMINFO_FILE}" ]]; then
    echo -e "${RED}Error: ${TERMINFO_FILE} not found${NC}"
    exit 1
fi

# Check if tic command is available
if ! command -v tic &> /dev/null; then
    echo -e "${RED}Error: 'tic' command not found${NC}"
    echo "Please install ncurses utilities:"
    echo "  Ubuntu/Debian: sudo apt-get install ncurses-bin"
    echo "  macOS: Should be pre-installed (or: brew install ncurses)"
    echo "  Fedora/RHEL: sudo dnf install ncurses"
    exit 1
fi

# Parse arguments
SYSTEM_INSTALL=false
if [[ "$1" == "--system" ]]; then
    SYSTEM_INSTALL=true
fi

# Install terminfo
if [[ "${SYSTEM_INSTALL}" == true ]]; then
    echo -e "${YELLOW}Installing par-term terminfo system-wide...${NC}"

    # Check if we have sudo rights
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}Error: System installation requires sudo/root privileges${NC}"
        echo "Run: sudo $0 --system"
        exit 1
    fi

    tic -x "${TERMINFO_FILE}"

    echo -e "${GREEN}✓ Successfully installed par-term terminfo system-wide${NC}"
    echo "  Location: /usr/share/terminfo/ (or /usr/share/misc/terminfo/)"
else
    echo -e "${YELLOW}Installing par-term terminfo for current user...${NC}"

    # Create user terminfo directory if it doesn't exist
    mkdir -p ~/.terminfo

    tic -x -o ~/.terminfo "${TERMINFO_FILE}"

    echo -e "${GREEN}✓ Successfully installed par-term terminfo to ~/.terminfo${NC}"
fi

echo ""
echo "To use par-term, set your TERM environment variable:"
echo -e "  ${GREEN}export TERM=par-term${NC}"
echo -e "  ${GREEN}export COLORTERM=truecolor${NC}"
echo ""
echo "Add these lines to your ~/.bashrc or ~/.zshrc to make them permanent."
echo ""
echo "Verify installation with:"
echo "  infocmp par-term"
echo "  tput colors"

# Verify installation
echo ""
echo -e "${YELLOW}Verifying installation...${NC}"
if infocmp par-term &> /dev/null; then
    echo -e "${GREEN}✓ par-term terminfo is now available${NC}"

    # Show color count
    COLOR_COUNT=$(tput -T par-term colors 2>/dev/null || echo "unknown")
    echo "  Supported colors: ${COLOR_COUNT}"

    # Check for true color support
    if tput -T par-term setrgbf 255 128 0 &> /dev/null; then
        echo -e "  True color: ${GREEN}supported${NC}"
    fi
else
    echo -e "${RED}✗ Failed to verify par-term installation${NC}"
    exit 1
fi
