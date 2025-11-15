#!/usr/bin/env python3
"""
Improved basic usage example of par_term_emu
Shows terminal content with visual boundaries
"""

from par_term_emu_core_rust import Terminal


def print_terminal_with_border(term: Terminal, title: str = "Terminal"):
    """Print terminal content with a visible border"""
    cols, rows = term.size()

    print(f"\n{'=' * 80}")
    print(f"{title} ({cols}x{rows})")
    print("=" * 80)

    # Get content line by line
    for row in range(rows):
        line = term.get_line(row)
        if line and line.strip():  # Only show non-empty lines
            print(f"{row:2d}: |{line}|")

    print("=" * 80 + "\n")


def print_terminal_with_colors(term: Terminal, title: str = "Terminal with Colors"):
    """Print terminal content with ANSI color codes reconstructed"""
    cols, rows = term.size()

    print(f"\n{'=' * 80}")
    print(f"{title} ({cols}x{rows})")
    print("=" * 80)

    for row in range(rows):
        line_parts = []
        for col in range(cols):
            char = term.get_char(col, row) or " "

            # Get color information
            fg = term.get_fg_color(col, row)
            bg = term.get_bg_color(col, row)
            attrs = term.get_attributes(col, row)

            # Build ANSI escape sequence
            codes = []

            # Attributes
            if attrs.bold:
                codes.append("1")
            if attrs.italic:
                codes.append("3")
            if attrs.underline:
                codes.append("4")
            if attrs.reverse:
                codes.append("7")

            # Foreground color (only if not default white)
            if fg != (192, 192, 192):  # Not default white
                codes.append(f"38;2;{fg[0]};{fg[1]};{fg[2]}")

            # Background color (only if not default black)
            if bg != (0, 0, 0):  # Not default black
                codes.append(f"48;2;{bg[0]};{bg[1]};{bg[2]}")

            # Apply formatting if needed
            if codes:
                line_parts.append(f"\x1b[{';'.join(codes)}m{char}\x1b[0m")
            else:
                line_parts.append(char)

        line = "".join(line_parts).rstrip()
        if line:  # Only print non-empty lines
            print(f"{row:2d}: |{line}|")

    print("=" * 80 + "\n")


def main():
    # Create a terminal with 80 columns and 24 rows
    term = Terminal(80, 24)

    print("\n" + "=" * 80)
    print("PAR Terminal Emulator - Basic Usage Demo")
    print("=" * 80)
    print(f"Terminal created: {term!r}")
    print(f"Size: {term.size()}")

    # Write some plain text
    print("\n>>> Writing plain text...")
    term.process_str("Hello, World!\n")
    term.process_str("This is a terminal emulator.\n")

    print_terminal_with_border(term, "Plain Text Output")

    # Write colored text
    print(">>> Writing colored text...")
    term.process_str("\x1b[31mRed text\x1b[0m\n")
    term.process_str("\x1b[32mGreen text\x1b[0m\n")
    term.process_str("\x1b[34mBlue text\x1b[0m\n")

    print_terminal_with_border(term, "Text with Colors (raw)")
    print_terminal_with_colors(term, "Text with Colors (rendered)")

    # Write text with attributes
    print(">>> Writing text with attributes...")
    term.process_str("\x1b[1mBold text\x1b[0m\n")
    term.process_str("\x1b[3mItalic text\x1b[0m\n")
    term.process_str("\x1b[4mUnderlined text\x1b[0m\n")
    term.process_str("\x1b[1;4;31mBold Underlined Red\x1b[0m\n")

    print_terminal_with_colors(term, "Text with Attributes")

    # Check cursor position
    col, row = term.cursor_position()
    print(f"Final cursor position: column={col}, row={row}")

    # Get a specific line
    line = term.get_line(0)
    print(f"First line content: {line!r}")

    print("\n" + "=" * 80)
    print("Demo complete!")
    print("=" * 80 + "\n")


if __name__ == "__main__":
    main()
