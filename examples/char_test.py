#!/usr/bin/env python3
"""
Character-Specific Test

Testing if the issue is with specific characters when using FG+BG colors.

The ▄ character (U+2584, lower half block) causes artifacts.
Let's test other characters to narrow down the issue.

Usage:
    python examples/char_test.py
"""

import colorsys
import sys


def get_terminal_size() -> tuple[int, int]:
    """Get terminal size."""
    import shutil
    size = shutil.get_terminal_size()
    return (size.columns, size.lines)


def rgb_fg(r: int, g: int, b: int) -> str:
    return f"\x1b[38;2;{r};{g};{b}m"


def rgb_bg(r: int, g: int, b: int) -> str:
    return f"\x1b[48;2;{r};{g};{b}m"


def reset() -> str:
    return "\x1b[0m"


def gradient_row(cols: int, char: str, use_fg: bool = True) -> str:
    """Generate one row of gradient with the specified character."""
    output = []
    for x in range(cols):
        h = x / cols
        l = 0.4

        r1, g1, b1 = colorsys.hls_to_rgb(h, l, 1.0)
        r2, g2, b2 = colorsys.hls_to_rgb(h, l + 0.1, 1.0)

        if use_fg:
            output.append(rgb_fg(int(r2 * 255), int(g2 * 255), int(b2 * 255)))
        output.append(rgb_bg(int(r1 * 255), int(g1 * 255), int(b1 * 255)))
        output.append(char)
        output.append(reset())

    output.append("\n")
    return "".join(output)


def test_character(name: str, char: str, cols: int, rows: int, use_fg: bool = True) -> None:
    """Test a specific character."""
    print(f"--- {name}: '{char}' (U+{ord(char):04X}) {'FG+BG' if use_fg else 'BG only'} ---")

    # Generate enough rows to trigger scroll
    for _ in range(rows + 5):
        sys.stdout.write(gradient_row(cols, char, use_fg))
    sys.stdout.flush()

    print()
    input("Press Enter for next test...")
    print()


def main() -> None:
    cols, rows = get_terminal_size()
    print(f"Terminal size: {cols}x{rows}")
    print()
    print("Testing different characters with FG+BG colors to find the culprit.")
    print("Watch for artifacts during scroll.")
    print()
    input("Press Enter to start tests...")
    print()

    # Test 1: Space with BG only (known to work)
    test_character("Space (BG only)", " ", cols, rows, use_fg=False)

    # Test 2: Space with FG+BG
    test_character("Space (FG+BG)", " ", cols, rows, use_fg=True)

    # Test 3: Regular ASCII character with FG+BG
    test_character("Letter X (FG+BG)", "X", cols, rows, use_fg=True)

    # Test 4: Full block with FG+BG
    test_character("Full block █ (FG+BG)", "█", cols, rows, use_fg=True)

    # Test 5: Lower half block (the problematic one from Rich)
    test_character("Lower half ▄ (FG+BG)", "▄", cols, rows, use_fg=True)

    # Test 6: Upper half block
    test_character("Upper half ▀ (FG+BG)", "▀", cols, rows, use_fg=True)

    # Test 7: Lower half block with BG only
    test_character("Lower half ▄ (BG only)", "▄", cols, rows, use_fg=False)

    # Test 8: Light shade
    test_character("Light shade ░ (FG+BG)", "░", cols, rows, use_fg=True)

    # Test 9: Medium shade
    test_character("Medium shade ▒ (FG+BG)", "▒", cols, rows, use_fg=True)

    # Test 10: Solid colored emoji (wide char)
    test_character("Red square 🟥 (FG+BG)", "🟥", cols // 2, rows, use_fg=True)

    print("=== Test Complete ===")
    print()
    print("Which characters showed artifacts?")
    print("This will help identify if it's:")
    print("- Specific to block characters (▄ ▀ █)")
    print("- Any character with FG+BG")
    print("- Related to character width")


if __name__ == "__main__":
    main()
