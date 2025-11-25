#!/usr/bin/env python3
"""
BCE (Background Color Erase) Scroll Test

This test specifically checks for the BCE behavior during scroll:
When a line scrolls out and a new blank line appears at the bottom,
should that line be filled with:
  - The DEFAULT background color (correct for most terminals)
  - The CURRENT SGR background color (BCE behavior)

If xterm.js incorrectly uses BCE, new lines during scroll will
have the wrong background color.

Usage:
    python examples/bce_scroll_test.py
"""

import os
import sys
import time


def get_terminal_size() -> tuple[int, int]:
    """Get terminal size."""
    import shutil
    size = shutil.get_terminal_size()
    return (size.columns, size.lines)


def main() -> None:
    """Run BCE scroll test."""
    cols, rows = get_terminal_size()
    print(f"Terminal size: {cols}x{rows}")
    print()

    # Test 1: Simple scroll with active background color
    print("=== Test 1: Scroll with active background color ===")
    print("Setting RED background, then triggering scroll...")
    print("If BCE bug exists: new lines will have RED background")
    print()
    time.sleep(1)

    # Fill terminal to near bottom
    for i in range(rows - 5):
        print(f"Line {i + 1}")

    # Now set a RED background and output text that will cause scroll
    # \x1b[41m = red background, \x1b[0m = reset
    sys.stdout.write("\x1b[41m")  # Set red background
    sys.stdout.write("RED BG LINE 1 - this line has red background")
    sys.stdout.write("\x1b[0m")  # Reset
    sys.stdout.write("\n")

    # More lines to force scroll while red background might leak
    sys.stdout.write("\x1b[41m")  # Set red background again
    sys.stdout.write("RED BG LINE 2 - scroll should happen here")
    # DON'T reset before newline - this is the key test
    sys.stdout.write("\n")  # Newline while BG is still red

    # Now output normal text - if BCE bug, this line's beginning might be red
    sys.stdout.write("\x1b[0m")  # Reset now
    sys.stdout.write("NORMAL LINE - should have DEFAULT background")
    sys.stdout.write("\n")

    print()
    print("=== Test 2: Rapid output scroll test ===")
    print("Rapidly outputting colored content to trigger scroll...")
    print()
    time.sleep(1)

    # This mimics what Rich does - rapid colored output
    for i in range(rows + 5):
        # Progress through colors: red -> yellow -> green
        t = i / max(rows + 4, 1)
        if t < 0.5:
            t2 = t * 2
            r = 255
            g = int(255 * t2)
            b = 0
        else:
            t2 = (t - 0.5) * 2
            r = int(255 * (1 - t2))
            g = 255
            b = 0

        # Set background color and output
        sys.stdout.write(f"\x1b[48;2;{r};{g};{b}m")
        sys.stdout.write(f" Row {i+1:3d} ")
        sys.stdout.write("\x1b[0m")  # Reset after content but before newline
        sys.stdout.write(f" <- BG should be default here, not RGB({r},{g},{b})")
        sys.stdout.write("\n")
        sys.stdout.flush()

    print()
    print("=== Test 3: No reset before newline (worst case) ===")
    print("This is the worst case - color set, newline, NO reset...")
    print()
    time.sleep(1)

    for i in range(rows + 5):
        t = i / max(rows + 4, 1)
        if t < 0.5:
            t2 = t * 2
            r = 255
            g = int(255 * t2)
            b = 0
        else:
            t2 = (t - 0.5) * 2
            r = int(255 * (1 - t2))
            g = 255
            b = 0

        # Set background and output content
        sys.stdout.write(f"\x1b[48;2;{r};{g};{b}m")
        sys.stdout.write(f" Row {i+1:3d} - BG is RGB({r},{g},{b})")
        # NO RESET before newline - if BCE, rest of line gets this color
        sys.stdout.write("\n")
        sys.stdout.flush()

    # Final reset
    sys.stdout.write("\x1b[0m")
    print()
    print("=== Test Complete ===")
    print("Scroll up and check:")
    print("1. Are there colored blocks at the START of lines?")
    print("2. Does text after the colored section have default BG?")
    print()
    print("If artifacts appear at line starts during scroll,")
    print("this confirms a BCE-related rendering issue.")


if __name__ == "__main__":
    main()
