#!/bin/bash
# Comprehensive graphics scrolling test for all protocols

echo "=== Comprehensive Graphics Scroll Test ==="
echo ""

# Test function
test_protocol() {
    local protocol=$1
    local command=$2

    echo "========================================"
    echo "Testing: $protocol"
    echo "========================================"

    # Add context lines
    echo "Context line 1 (BEFORE image)"
    echo "Context line 2 (BEFORE image)"
    echo "Context line 3 (BEFORE image)"

    # Display image
    echo ">>> Displaying image with: $command"
    eval "$command"

    # Add context lines
    echo "Context line 1 (AFTER image)"
    echo "Context line 2 (AFTER image)"
    echo "Context line 3 (AFTER image)"

    echo ""
    echo ">>> Now scrolling with 40 lines..."
    echo ">>> Watch if the image stays aligned with 'BEFORE' and 'AFTER' markers"
    echo ""

    # Scroll
    for i in {1..40}; do
        echo "Scroll line $i"
    done

    echo ""
    echo ">>> Expected: Image should maintain relative position to BEFORE/AFTER markers"
    echo ">>> If bug present: Image would drift away from markers (scroll faster)"
    echo ""
    read -p "Press ENTER to continue to next test..."
    clear
}

# Test 1: Kitty graphics (if available)
if command -v kitty &> /dev/null; then
    test_protocol "Kitty Graphics (file transmission)" \
        "kitty +kitten icat --align=left /Users/probello/Repos/par-term/images/snake_tui.png"
else
    echo "SKIP: kitty not found"
fi

# Test 2: Sixel (if available)
if command -v img2sixel &> /dev/null; then
    test_protocol "Sixel Graphics" \
        "img2sixel /Users/probello/Repos/par-term/images/snake_tui.png"
else
    echo "SKIP: img2sixel not found"
fi

# Test 3: iTerm2 (if available)
if command -v imgcat &> /dev/null; then
    test_protocol "iTerm2 Graphics" \
        "imgcat /Users/probello/Repos/par-term/images/snake_tui.png"
else
    echo "SKIP: imgcat not found"
fi

echo "=== All tests complete ==="
