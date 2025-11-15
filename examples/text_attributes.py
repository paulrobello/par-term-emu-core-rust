#!/usr/bin/env python3
"""
Text attributes demonstration (bold, italic, underline, etc.)
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 30)

    print("=== Text Attributes Demo ===\n")

    # Demonstrate various text attributes
    term.process_str("Text Attributes:\n\n")

    term.process_str("\x1b[1mBold text\x1b[0m\n")
    term.process_str("\x1b[2mDim text\x1b[0m\n")
    term.process_str("\x1b[3mItalic text\x1b[0m\n")
    term.process_str("\x1b[4mUnderlined text\x1b[0m\n")
    term.process_str("\x1b[5mBlinking text\x1b[0m\n")
    term.process_str("\x1b[7mReverse video\x1b[0m\n")
    term.process_str("\x1b[8mHidden text\x1b[0m\n")
    term.process_str("\x1b[9mStrikethrough text\x1b[0m\n")

    term.process_str("\n")

    # Combined attributes
    term.process_str("Combined attributes:\n")
    term.process_str("\x1b[1;4;31mBold, underlined, red text\x1b[0m\n")
    term.process_str("\x1b[3;32mItalic green text\x1b[0m\n")
    term.process_str("\x1b[1;3;4;33mBold italic underlined yellow\x1b[0m\n")

    # Print the terminal content with formatting
    print_terminal_content(term, show_colors=True)

    # Check attributes at specific positions
    print("\n=== Attribute Information ===")

    # Write a bold text and check its attributes
    term.reset()
    term.process_str("\x1b[1;4mBold and underlined\x1b[0m")

    attrs = term.get_attributes(0, 0)
    if attrs:
        print(f"Attributes at (0, 0): {attrs}")
        print(f"  Bold: {attrs.bold}")
        print(f"  Italic: {attrs.italic}")
        print(f"  Underline: {attrs.underline}")


if __name__ == "__main__":
    main()
