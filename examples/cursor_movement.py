#!/usr/bin/env python3
"""
Cursor movement and positioning demo
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(40, 20)

    print("=== Cursor Movement Demo ===\n")

    # Draw a box using cursor positioning
    term.process_str("\x1b[2;5H")  # Move to row 2, col 5
    term.process_str("┌" + "─" * 20 + "┐")

    for i in range(3, 8):
        term.process_str(f"\x1b[{i};5H│")
        term.process_str(f"\x1b[{i};26H│")

    term.process_str("\x1b[8;5H")
    term.process_str("└" + "─" * 20 + "┘")

    # Write text in the center
    term.process_str("\x1b[5;10H")
    term.process_str("Hello, Terminal!")

    # Move cursor around
    term.process_str("\x1b[10;1H")
    term.process_str("Moving cursor: ")

    term.process_str("\x1b[2C")  # Move right 2 positions
    term.process_str("→→ ")

    term.process_str("\x1b[3D")  # Move left 3 positions
    term.process_str("←←← ")

    term.process_str("\x1b[11;1H")
    term.process_str("Cursor position: ")
    col, row = term.cursor_position()
    term.process_str(f"col={col}, row={row}")

    # Demonstrate cursor visibility
    term.process_str("\x1b[?25l")  # Hide cursor
    term.process_str("\x1b[12;1H")
    term.process_str(f"Cursor visible: {term.cursor_visible()}")

    term.process_str("\x1b[?25h")  # Show cursor

    # Print the result
    print_terminal_content(term, show_colors=True)


if __name__ == "__main__":
    main()
