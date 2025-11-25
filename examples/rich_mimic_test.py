#!/usr/bin/env python3
"""
Rich Mimic Test

This test mimics exactly how Rich outputs its gradient:
1. Uses the ▄ (lower half block) character like Rich's ColorBox
2. Uses both foreground AND background colors
3. Outputs the entire content in one large write (like Rich does)

This should help identify if the issue is specific to:
- The ▄ character
- Using both FG and BG colors together
- Large batch writes
- Something else in Rich's rendering

Usage:
    python examples/rich_mimic_test.py
"""

import colorsys
import os
import sys


def get_terminal_size() -> tuple[int, int]:
    """Get terminal size."""
    import shutil
    size = shutil.get_terminal_size()
    return (size.columns, size.lines)


def rgb_fg(r: int, g: int, b: int) -> str:
    """Set RGB foreground color."""
    return f"\x1b[38;2;{r};{g};{b}m"


def rgb_bg(r: int, g: int, b: int) -> str:
    """Set RGB background color."""
    return f"\x1b[48;2;{r};{g};{b}m"


def reset() -> str:
    """Reset all attributes."""
    return "\x1b[0m"


def main() -> None:
    """Run Rich mimic test."""
    cols, rows = get_terminal_size()
    print(f"Terminal size: {cols}x{rows}")
    print()

    # Test 1: Mimic Rich's ColorBox exactly
    # Rich uses ▄ with both FG and BG colors, outputs in one batch
    print("=== Test 1: Mimic Rich ColorBox (single batch) ===")
    print("Using ▄ character with FG+BG colors, output all at once...")
    print()

    # Build entire output in memory first (like Rich does)
    output = []

    # Generate enough rows to trigger scroll
    num_rows = rows + 10

    for y in range(num_rows):
        for x in range(cols):
            h = x / cols
            l = 0.1 + ((y / num_rows) * 0.7)

            # Two slightly different lightness values for FG and BG
            r1, g1, b1 = colorsys.hls_to_rgb(h, l, 1.0)
            r2, g2, b2 = colorsys.hls_to_rgb(h, l + 0.07, 1.0)

            # FG and BG like Rich does
            output.append(rgb_fg(int(r2 * 255), int(g2 * 255), int(b2 * 255)))
            output.append(rgb_bg(int(r1 * 255), int(g1 * 255), int(b1 * 255)))
            output.append("▄")
            output.append(reset())  # Rich resets after each segment

        output.append("\n")

    # Write ALL at once (like Rich does)
    sys.stdout.write("".join(output))
    sys.stdout.flush()

    print()
    print("=== Test 2: Same but character-by-character ===")
    print("Using ▄ character with FG+BG colors, output one at a time...")
    print()

    import time

    for y in range(10):  # Fewer rows for this test
        for x in range(cols):
            h = x / cols
            l = 0.3

            r1, g1, b1 = colorsys.hls_to_rgb(h, l, 1.0)
            r2, g2, b2 = colorsys.hls_to_rgb(h, l + 0.07, 1.0)

            sys.stdout.write(rgb_fg(int(r2 * 255), int(g2 * 255), int(b2 * 255)))
            sys.stdout.write(rgb_bg(int(r1 * 255), int(g1 * 255), int(b1 * 255)))
            sys.stdout.write("▄")
            sys.stdout.write(reset())
            sys.stdout.flush()

        sys.stdout.write("\n")
        sys.stdout.flush()

    print()
    print("=== Test 3: Space only (no FG color) in single batch ===")
    print("Using space with BG color only, output all at once...")
    print()

    output = []
    for y in range(num_rows):
        for x in range(cols):
            h = x / cols
            l = 0.3 + ((y / num_rows) * 0.5)

            r, g, b = colorsys.hls_to_rgb(h, l, 1.0)

            output.append(rgb_bg(int(r * 255), int(g * 255), int(b * 255)))
            output.append(" ")
            output.append(reset())

        output.append("\n")

    sys.stdout.write("".join(output))
    sys.stdout.flush()

    print()
    print("=== Test Complete ===")
    print()
    print("Compare the tests:")
    print("- Test 1 (▄ + FG+BG, batch) - mimics Rich exactly")
    print("- Test 2 (▄ + FG+BG, char-by-char)")
    print("- Test 3 (space + BG only, batch)")
    print()
    print("If only Test 1 has artifacts, the issue is batch + special char")
    print("If Test 1 and 3 have artifacts, the issue is batch writing")
    print("If none have artifacts, the issue is something else in Rich")


if __name__ == "__main__":
    main()
