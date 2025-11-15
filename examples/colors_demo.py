#!/usr/bin/env python3
"""
Color demonstration using par_term_emu
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(100, 30)

    print("=== ANSI Color Demo ===\n")

    # Basic 16 colors
    term.process_str("Basic 16 colors:\n")
    for i in range(16):
        term.process_str(f"\x1b[{30 + (i % 8)}m")
        if i >= 8:
            term.process_str("\x1b[1m")  # Bold for bright colors
        term.process_str(f"Color {i:2d} ")
        term.process_str("\x1b[0m")
        if (i + 1) % 8 == 0:
            term.process_str("\n")

    term.process_str("\n")

    # 256-color palette (sample)
    term.process_str("256-color palette (sample):\n")
    for i in range(16, 232, 36):
        for j in range(6):
            color_idx = i + j * 6
            term.process_str(f"\x1b[38;5;{color_idx}m█\x1b[0m")
        term.process_str("\n")

    term.process_str("\n")

    # Grayscale ramp
    term.process_str("Grayscale ramp:\n")
    for i in range(232, 256):
        term.process_str(f"\x1b[38;5;{i}m█\x1b[0m")
    term.process_str("\n\n")

    # RGB colors (24-bit)
    term.process_str("RGB (24-bit) colors:\n")
    for r in range(0, 256, 51):
        for g in range(0, 256, 51):
            for b in range(0, 256, 51):
                term.process_str(f"\x1b[38;2;{r};{g};{b}m█\x1b[0m")
        term.process_str("\n")

    # Print the terminal content with colors
    print_terminal_content(term, show_colors=True)

    # Demonstrate color retrieval
    print("\n=== Color Information ===")
    term.process_str("\n\x1b[38;2;255;0;0mRed\x1b[0m")

    # Get color information for a colored cell
    if fg_color := term.get_fg_color(0, term.cursor_position()[1] - 1):
        r, g, b = fg_color
        print(f"Foreground color at last line: RGB({r}, {g}, {b})")


if __name__ == "__main__":
    main()
