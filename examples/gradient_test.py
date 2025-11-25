#!/usr/bin/env python3
"""
Minimal Gradient Test for Debugging Terminal Wrap Behavior

This script outputs a simple gradient bar that fills exactly the terminal width.
It can be used to verify that both xterm.js (streaming) and TUI correctly handle
line wrapping at the terminal width boundary.

Usage:
    python examples/gradient_test.py

The test outputs:
1. Terminal size (as queried via ioctl)
2. A gradient bar that should fill exactly one line
3. A marker character on the next line to verify no unwanted wrap occurred

If the gradient wraps incorrectly, you'll see part of the gradient on the second
line before the marker character.
"""

import os
import sys


def get_terminal_size() -> tuple[int, int]:
    """Get terminal size using multiple methods for debugging."""
    # Method 1: os.get_terminal_size()
    try:
        size = os.get_terminal_size()
        print(f"os.get_terminal_size(): {size.columns}x{size.lines}")
    except OSError as e:
        print(f"os.get_terminal_size() failed: {e}")
        size = None

    # Method 2: shutil
    import shutil

    try:
        shutil_size = shutil.get_terminal_size()
        print(f"shutil.get_terminal_size(): {shutil_size.columns}x{shutil_size.lines}")
    except Exception as e:
        print(f"shutil.get_terminal_size() failed: {e}")
        shutil_size = None

    # Method 3: Direct ioctl (Unix only)
    if sys.platform != "win32":
        try:
            import fcntl
            import struct
            import termios

            result = fcntl.ioctl(sys.stdout.fileno(), termios.TIOCGWINSZ, b"\x00" * 8)
            rows, cols, _, _ = struct.unpack("HHHH", result)
            print(f"Direct ioctl TIOCGWINSZ: {cols}x{rows}")
        except Exception as e:
            print(f"Direct ioctl failed: {e}")

    # Use shutil as fallback since it handles edge cases
    if shutil_size:
        return (shutil_size.columns, shutil_size.lines)
    if size:
        return (size.columns, size.lines)
    return (80, 24)  # fallback


def rgb_to_ansi_bg(r: int, g: int, b: int) -> str:
    """Return ANSI escape sequence for 24-bit background color."""
    return f"\x1b[48;2;{r};{g};{b}m"


def reset_style() -> str:
    """Return ANSI reset sequence."""
    return "\x1b[0m"


def gradient_bar(cols: int) -> str:
    """
    Generate a gradient bar that fills exactly `cols` characters.

    The gradient goes from red (left) through yellow to green (right).
    Each character is a space with a background color.
    """
    result = []
    for i in range(cols):
        # Progress from 0.0 to 1.0
        t = i / max(cols - 1, 1)

        # Red to Yellow (0.0 to 0.5)
        if t < 0.5:
            t2 = t * 2  # 0.0 to 1.0
            r = 255
            g = int(255 * t2)
            b = 0
        # Yellow to Green (0.5 to 1.0)
        else:
            t2 = (t - 0.5) * 2  # 0.0 to 1.0
            r = int(255 * (1 - t2))
            g = 255
            b = 0

        result.append(rgb_to_ansi_bg(r, g, b) + " ")

    result.append(reset_style())
    return "".join(result)


def main() -> None:
    """Run the gradient test."""
    print("=== Gradient Wrap Test ===")
    print()

    cols, rows = get_terminal_size()
    print()
    print(f"Using terminal size: {cols}x{rows}")
    print()

    # Test 0: Many gradient rows to force scroll during gradient output
    # Include left-side labels like Rich does
    print("--- Test 0: Force scroll during gradient (like Rich) ---")
    print("Outputting gradient rows with left labels to force scroll...")
    print()

    # Labels similar to Rich's output
    labels = [
        "Colors",
        "  ✓ 4-bit color",
        "  ✓ 8-bit color",
        "  ✓ Truecolor (16.7 million)",
        "  ✓ Dumb terminals",
        "",
        "  ✓ Automatic color conversion",
        "Styles",
        "  Bold",
        "  Dim",
        "  Italic",
        "  Underline",
        "  Strikethrough",
        "  Reverse",
        "Progress",
        "  Loading...",
        "Tables",
        "  Row 1",
        "  Row 2",
        "  Row 3",
    ]

    # Pad labels to consistent width
    label_width = 30
    num_gradient_rows = rows + 10

    for row_num in range(num_gradient_rows):
        # Get label for this row (cycle through labels)
        label = labels[row_num % len(labels)]
        padded_label = label.ljust(label_width)

        # Calculate remaining width for gradient
        gradient_width = cols - label_width
        if gradient_width > 0:
            gradient = gradient_bar(gradient_width)
            sys.stdout.write(padded_label)
            sys.stdout.write(gradient)
            sys.stdout.write("\n")
        else:
            sys.stdout.write(padded_label + "\n")
        sys.stdout.flush()

    print()
    print(f"▶ Marker after {num_gradient_rows} gradient rows with labels")
    print("If artifact appears, scroll happened during styled output")
    print()

    print("--- Test 1: Full-width gradient bar ---")
    print("Expect: One line of gradient from red to green")
    print("If wrap issue: Part of gradient appears on next line")
    print()

    # Output exactly cols characters (gradient bar)
    gradient = gradient_bar(cols)
    print(gradient)

    # Output a marker to see where the cursor ends up
    print("▶ Marker (should be at start of this line)")
    print()

    print("--- Test 2: Width - 1 gradient bar ---")
    print("Expect: Gradient fills all but last column")
    print()

    gradient_minus_1 = gradient_bar(cols - 1)
    print(gradient_minus_1)
    print("▶ Marker")
    print()

    print("--- Test 3: Width + 1 gradient bar (force wrap) ---")
    print("Expect: Gradient fills line + 1 char wraps to next line")
    print()

    gradient_plus_1 = gradient_bar(cols + 1)
    print(gradient_plus_1)
    print("▶ Marker (should be AFTER the 1 wrapped char)")
    print()

    print("--- Test 4: Verify with Rich (if available) ---")
    try:
        from rich.console import Console
        from rich.text import Text

        console = Console(force_terminal=True)
        console_width = console.width
        print(f"Rich console.width: {console_width}")

        # Create a gradient text that fills the width
        text = Text()
        for i in range(console_width):
            t = i / max(console_width - 1, 1)
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
            text.append(" ", style=f"on rgb({r},{g},{b})")

        console.print(text)
        console.print("▶ Rich marker")
    except ImportError:
        print("Rich not installed, skipping Rich test")
    print()

    print("=== Test Complete ===")
    print("If all markers are at the start of their lines, wrap behavior is correct.")


if __name__ == "__main__":
    main()
