#!/usr/bin/env python3
"""
Scroll Timing Test

This test isolates the exact moment when scroll happens during styled output
to determine if it's a timing/batching issue vs a rendering issue.

The key question: when scroll occurs mid-line during gradient output,
is the cursor position preserved correctly?

Usage:
    python examples/scroll_timing_test.py
"""

import os
import sys
import time


def get_terminal_size() -> tuple[int, int]:
    """Get terminal size."""
    import shutil
    size = shutil.get_terminal_size()
    return (size.columns, size.lines)


def rgb_bg(r: int, g: int, b: int) -> str:
    """Set RGB background color."""
    return f"\x1b[48;2;{r};{g};{b}m"


def reset() -> str:
    """Reset all attributes."""
    return "\x1b[0m"


def cursor_pos() -> str:
    """Request cursor position (DSR 6)."""
    return "\x1b[6n"


def main() -> None:
    """Run scroll timing test."""
    cols, rows = get_terminal_size()
    print(f"Terminal size: {cols}x{rows}")
    print()

    # Test 1: Output styled content character by character to see exact scroll point
    print("=== Test 1: Character-by-character styled output ===")
    print("Watch for the exact moment scroll happens...")
    print()
    time.sleep(1)

    # Fill to near bottom
    for i in range(rows - 3):
        print(f"Fill line {i + 1}")

    print("Now outputting gradient that will trigger scroll:")
    print()

    # Output gradient one character at a time with delays
    for col in range(cols):
        # Red gradient
        r = 255 - int((col / cols) * 200)
        g = int((col / cols) * 100)
        b = 0

        sys.stdout.write(rgb_bg(r, g, b))
        sys.stdout.write("█")
        sys.stdout.flush()
        # Small delay to see what's happening
        time.sleep(0.01)

    # Now the critical moment - newline while background is set
    sys.stdout.write("\n")
    sys.stdout.flush()

    # Continue on next line (after scroll)
    for col in range(cols):
        r = 255 - int((col / cols) * 200)
        g = int((col / cols) * 100)
        b = 0

        sys.stdout.write(rgb_bg(r, g, b))
        sys.stdout.write("█")
        sys.stdout.flush()
        time.sleep(0.01)

    sys.stdout.write(reset())
    sys.stdout.write("\n\n")

    print("=== Test 2: Fast output (realistic scenario) ===")
    time.sleep(1)

    # Now test at full speed
    for row in range(rows + 5):
        for col in range(cols):
            r = 255 - int((col / cols) * 200)
            g = int((col / cols) * 100) + int((row / (rows + 5)) * 155)
            b = int((row / (rows + 5)) * 100)

            sys.stdout.write(rgb_bg(r, g, b))
            sys.stdout.write(" ")

        sys.stdout.write(reset())
        sys.stdout.write("\n")
        sys.stdout.flush()

    print()
    print("=== Test 3: With explicit SGR reset before newline ===")
    print("This should NOT have artifacts if BCE is the issue...")
    time.sleep(1)

    for row in range(rows + 5):
        for col in range(cols):
            r = 255 - int((col / cols) * 200)
            g = int((col / cols) * 100) + int((row / (rows + 5)) * 155)
            b = int((row / (rows + 5)) * 100)

            sys.stdout.write(rgb_bg(r, g, b))
            sys.stdout.write(" ")

        # RESET BEFORE NEWLINE
        sys.stdout.write(reset())
        sys.stdout.write("\n")
        sys.stdout.flush()

    print()
    print("=== Test Complete ===")
    print()
    print("Compare Test 2 vs Test 3:")
    print("- If Test 3 has no artifacts but Test 2 does -> BCE issue")
    print("- If both have artifacts -> cursor/position issue")
    print("- If neither has artifacts -> timing issue resolved by explicit reset")


if __name__ == "__main__":
    main()
