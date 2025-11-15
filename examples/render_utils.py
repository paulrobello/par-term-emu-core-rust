"""
Utility functions for rendering terminal content with colors and attributes.

This module provides helper functions to display terminal emulator content
with proper ANSI color codes reconstructed from cell attributes.
"""

from par_term_emu_core_rust import Terminal


def render_line_with_colors(term: Terminal, row: int) -> str:
    """
    Render a single line with ANSI color codes reconstructed from cell attributes.

    Args:
        term: Terminal instance
        row: Row number to render

    Returns:
        String with ANSI escape codes for colors and attributes
    """
    cols, rows = term.size()
    if row >= rows:
        return ""

    line_parts = []
    prev_codes = []

    for col in range(cols):
        char = term.get_char(col, row) or " "

        # Get color and attribute information
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
        if attrs.blink:
            codes.append("5")
        if attrs.reverse:
            codes.append("7")
        if attrs.hidden:
            codes.append("8")
        if attrs.strikethrough:
            codes.append("9")

        # Foreground color (only if not default white)
        if fg != (192, 192, 192):  # Not default white
            codes.append(f"38;2;{fg[0]};{fg[1]};{fg[2]}")

        # Background color (only if not default black)
        if bg != (0, 0, 0):  # Not default black
            codes.append(f"48;2;{bg[0]};{bg[1]};{bg[2]}")

        # Only emit escape codes if they changed
        if codes != prev_codes:
            if codes:
                line_parts.append(f"\x1b[{';'.join(codes)}m{char}")
            else:
                line_parts.append(f"\x1b[0m{char}")
            prev_codes = codes
        else:
            line_parts.append(char)

    # Reset at end of line
    result = "".join(line_parts)
    if prev_codes:
        result += "\x1b[0m"

    return result.rstrip()


def print_terminal_content(
    term: Terminal,
    show_colors: bool = True,
    show_empty: bool = False,
    title: str | None = None,
):
    """
    Print terminal content, optionally with colors and formatting.

    Args:
        term: Terminal instance
        show_colors: If True, render with ANSI color codes
        show_empty: If True, show empty lines
        title: Optional title to print before content
    """
    cols, rows = term.size()

    if title:
        print(f"\n{'=' * 80}")
        print(title)
        print("=" * 80)

    for row in range(rows):
        if show_colors:
            line = render_line_with_colors(term, row)
        else:
            line = term.get_line(row)
            if line:
                line = line.rstrip()

        # Skip empty lines unless requested
        if line or show_empty:
            print(line)

    if title:
        print("=" * 80 + "\n")


def print_terminal_with_border(term: Terminal, title: str | None = None):
    """
    Print terminal content with a visual border.

    Args:
        term: Terminal instance
        title: Optional title
    """
    cols, rows = term.size()

    if title:
        print(f"\n{title}")
    print(f"┌{'─' * cols}┐")

    for row in range(rows):
        line = render_line_with_colors(term, row)
        # Pad to full width
        line = line + " " * (cols - len(line.encode("utf-8")))  # Account for ANSI codes
        print(f"│{line}│")

    print(f"└{'─' * cols}┘\n")


def print_cell_info(term: Terminal, col: int, row: int):
    """
    Print detailed information about a specific cell.

    Args:
        term: Terminal instance
        col: Column number
        row: Row number
    """
    char = term.get_char(col, row)
    fg = term.get_fg_color(col, row)
    bg = term.get_bg_color(col, row)
    attrs = term.get_attributes(col, row)

    print(f"Cell ({col}, {row}):")
    print(f"  Character: {char!r}")
    print(f"  Foreground: RGB{fg}")
    print(f"  Background: RGB{bg}")
    print(f"  Bold: {attrs.bold}")
    print(f"  Italic: {attrs.italic}")
    print(f"  Underline: {attrs.underline}")
    print(f"  Blink: {attrs.blink}")
    print(f"  Reverse: {attrs.reverse}")
    print(f"  Hidden: {attrs.hidden}")
    print(f"  Strikethrough: {attrs.strikethrough}")
