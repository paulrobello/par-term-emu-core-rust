#!/usr/bin/env bash
# Test script for par-term terminfo capabilities
# Usage: TERM=par-term ./test_terminfo.sh

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo "======================================================"
echo "PAR Terminal Emulator - Terminfo Capability Test"
echo "======================================================"
echo ""

# Check current TERM
echo -e "${BLUE}Current TERM:${NC} ${TERM}"
echo ""

# Verify terminfo exists
if ! infocmp "${TERM}" &> /dev/null; then
    echo -e "${RED}Error: Terminfo for '${TERM}' not found${NC}"
    echo "Please install the terminfo first:"
    echo "  ./terminfo/install.sh"
    exit 1
fi

echo -e "${GREEN}✓ Terminfo found${NC}"
echo ""

# Test basic capabilities
echo "Testing basic capabilities:"
echo "----------------------------------------------------"

# Color support
COLORS=$(tput colors 2>/dev/null || echo "0")
echo -e "  Colors supported: ${GREEN}${COLORS}${NC}"

# Cursor movement
if tput cup 0 0 &> /dev/null; then
    echo -e "  Cursor positioning: ${GREEN}supported${NC}"
fi

# Clear screen
if tput clear &> /dev/null; then
    echo -e "  Clear screen: ${GREEN}supported${NC}"
fi

# Bold text
if tput bold &> /dev/null; then
    echo -e "  Bold text: ${GREEN}supported${NC}"
fi

# Underline
if tput smul &> /dev/null; then
    echo -e "  Underline: ${GREEN}supported${NC}"
fi

# Standout/reverse
if tput smso &> /dev/null; then
    echo -e "  Reverse video: ${GREEN}supported${NC}"
fi

echo ""

# Test 256 colors
echo "Testing 256-color palette:"
echo "----------------------------------------------------"
for i in {0..15}; do
    tput setaf "$i"
    printf "█"
done
tput sgr0
echo ""
echo ""

# Test true color (if supported)
echo "Testing 24-bit true color (RGB):"
echo "----------------------------------------------------"
if tput setrgbf 255 0 0 &> /dev/null; then
    # Red
    tput setrgbf 255 0 0
    printf "Red "
    tput sgr0

    # Green
    tput setrgbf 0 255 0
    printf "Green "
    tput sgr0

    # Blue
    tput setrgbf 0 0 255
    printf "Blue "
    tput sgr0

    # Orange
    tput setrgbf 255 128 0
    printf "Orange "
    tput sgr0

    # Purple
    tput setrgbf 128 0 255
    printf "Purple "
    tput sgr0

    echo ""
    echo -e "  ${GREEN}✓ True color supported${NC}"
else
    echo -e "  ${YELLOW}⚠ True color not detected${NC}"
fi
echo ""

# Test text attributes
echo "Testing text attributes:"
echo "----------------------------------------------------"
tput sgr0
echo -n "  Normal text | "
tput bold
echo -n "Bold text"
tput sgr0
echo -n " | "
tput dim
echo -n "Dim text"
tput sgr0
echo ""

tput smul
echo -n "  Underline"
tput rmul
echo -n " | "
tput sitm 2>/dev/null || true
echo -n "Italic"
tput ritm 2>/dev/null || true
echo -n " | "
tput rev
echo -n "Reverse"
tput sgr0
echo ""

echo ""

# Test cursor visibility
echo "Testing cursor control:"
echo "----------------------------------------------------"
echo "  Current cursor position: $(tput cup 0 0 && echo -n '(via tput)' || echo 'unavailable')"
if tput civis &> /dev/null && tput cnorm &> /dev/null; then
    echo -e "  Cursor visibility control: ${GREEN}supported${NC}"
fi
echo ""

# Test special features
echo "Testing extended capabilities:"
echo "----------------------------------------------------"

# Bracketed paste
if tput BD &> /dev/null 2>&1 && tput BE &> /dev/null 2>&1; then
    echo -e "  Bracketed paste mode: ${GREEN}supported${NC}"
else
    echo -e "  Bracketed paste mode: ${YELLOW}not detected${NC}"
fi

# Mouse support
if tput kmous &> /dev/null; then
    echo -e "  Mouse support: ${GREEN}supported${NC}"
fi

# Alternate screen
if tput smcup &> /dev/null && tput rmcup &> /dev/null; then
    echo -e "  Alternate screen: ${GREEN}supported${NC}"
fi

echo ""
echo "======================================================"
echo -e "${GREEN}Terminfo test complete!${NC}"
echo "======================================================"
