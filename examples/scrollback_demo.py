#!/usr/bin/env python3
"""
Scrollback buffer demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a small terminal to demonstrate scrolling
    term = Terminal(40, 10, scrollback=100)

    print("=== Scrollback Demo ===\n")
    print(f"Terminal size: {term.size()}")
    print(f"Scrollback size: {term.scrollback_len()}\n")

    # Write more lines than the terminal height
    for i in range(1, 25):
        term.process_str(f"Line {i:2d}: This is a test line\n")

    # Print current visible content
    print("Current visible content:")
    print("-" * 40)
    print_terminal_content(term, show_colors=True)
    print("-" * 40)

    # Show scrollback buffer
    scrollback = term.scrollback()
    print(f"\nScrollback buffer has {len(scrollback)} lines:")
    print("-" * 40)
    for i, line in enumerate(scrollback, 1):
        print(f"Scrollback {i:2d}: {line.rstrip()}")
    print("-" * 40)

    # Demonstrate resize
    print("\nResizing terminal to 40x15...")
    term.resize(40, 15)
    print(f"New size: {term.size()}")


if __name__ == "__main__":
    main()
