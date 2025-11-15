#!/usr/bin/env python3
"""
Example demonstrating VT420 rectangle operations.

Rectangle operations allow efficient manipulation of rectangular regions
of the terminal screen: filling, copying, and erasing.

Usage:
    python examples/rectangle_operations.py
"""

from par_term_emu_core_rust import Terminal


def print_screen(term, title):
    """Print terminal content with a title"""
    print(f"\n{title}")
    print("=" * 60)
    content = term.content()
    lines = content.split("\n")
    for i, line in enumerate(lines[:12], 1):  # Show first 12 lines
        print(f"{i:2d}: {line}")
    print("=" * 60)


def demo_decfra():
    """Demonstrate DECFRA - Fill Rectangular Area"""
    print("\n### DECFRA - Fill Rectangular Area ###")
    term = Terminal(60, 15)

    # Clear screen and draw a border
    term.process_str("\x1b[2J\x1b[H")
    term.process_str("Rectangle Fill Demo")

    # Fill a rectangle with '*' characters at (5,5) to (10,40)
    # ESC [ Pc ; Pt ; Pl ; Pb ; Pr $ x
    # Pc=42 (ASCII '*'), Pt=5, Pl=5, Pb=10, Pr=40
    term.process_str("\x1b[42;5;5;10;40$x")

    print_screen(term, "After filling rectangle (5,5) to (10,40) with '*'")

    # Fill another rectangle with colored characters
    term.process_str("\x1b[31m")  # Red foreground
    # Fill rectangle (7,15) to (8,30) with '#'
    term.process_str("\x1b[35;7;15;8;30$x")  # ASCII 35 = '#'

    print_screen(term, "After filling (7,15) to (8,30) with red '#'")
    return term


def demo_deccra():
    """Demonstrate DECCRA - Copy Rectangular Area"""
    print("\n\n### DECCRA - Copy Rectangular Area ###")
    term = Terminal(60, 15)

    # Clear screen and write source text
    term.process_str("\x1b[2J\x1b[H")
    term.process_str("Rectangle Copy Demo\r\n\r\n")

    # Write a pattern in source area
    term.process_str("\x1b[5;10H")  # Move to (5,10)
    term.process_str("SOURCE")

    print_screen(term, "Original content with 'SOURCE' at (5,10)")

    # Copy rectangle from (5,10) to (5,15) to destination (8,25)
    # ESC [ Pts ; Pls ; Pbs ; Prs ; Pps ; Ptd ; Pld ; Ppd $ v
    # Source: row 5, cols 10-15, page 1
    # Dest: row 8, col 25, page 1
    term.process_str("\x1b[5;10;5;15;1;8;25;1$v")

    print_screen(term, "After copying (5,10)-(5,15) to (8,25)")

    # Copy to multiple locations
    term.process_str("\x1b[5;10;5;15;1;10;40;1$v")
    term.process_str("\x1b[5;10;5;15;1;12;10;1$v")

    print_screen(term, "After copying to additional locations")
    return term


def demo_decsera():
    """Demonstrate DECSERA - Selective Erase Rectangular Area"""
    print("\n\n### DECSERA - Selective Erase Rectangular Area ###")
    term = Terminal(60, 15)

    # Clear screen and fill with pattern
    term.process_str("\x1b[2J\x1b[H")
    term.process_str("Rectangle Erase Demo\r\n")

    # Fill screen with 'X' characters
    for row in range(4, 13):
        for col in range(5, 50):
            term.process_str(f"\x1b[{row};{col}HX")

    print_screen(term, "Screen filled with 'X' characters")

    # Erase rectangle (6,10) to (9,30)
    # ESC [ Pt ; Pl ; Pb ; Pr $ {
    term.process_str("\x1b[6;10;9;30${")

    print_screen(term, "After erasing rectangle (6,10) to (9,30)")

    # Erase another rectangle
    term.process_str("\x1b[7;35;10;45${")

    print_screen(term, "After erasing rectangle (7,35) to (10,45)")
    return term


def demo_combined():
    """Demonstrate combining rectangle operations"""
    print("\n\n### Combined Rectangle Operations ###")
    term = Terminal(60, 15)

    # Clear screen
    term.process_str("\x1b[2J\x1b[H")
    term.process_str("Combined Operations Demo\r\n")

    # 1. Fill background with dots
    term.process_str("\x1b[2m")  # Dim
    term.process_str("\x1b[46;4;5;12;55$x")  # Fill with '.' (ASCII 46)

    # 2. Fill a colored box
    term.process_str("\x1b[0m\x1b[44m")  # Reset, blue background
    term.process_str("\x1b[32;6;10;10;40$x")  # Fill with space (ASCII 32)

    # 3. Write text in the box
    term.process_str("\x1b[0m\x1b[33m")  # Reset, yellow foreground
    term.process_str("\x1b[7;15HRECTANGLE")
    term.process_str("\x1b[8;15HOPERATIONS")

    print_screen(term, "After combining fill operations and text")

    # 4. Copy the box to another location
    term.process_str("\x1b[6;10;10;40;1;6;45;1$v")

    print_screen(term, "After copying the box to (6,45)")

    # 5. Erase part of the copied box
    term.process_str("\x1b[7;47;9;52${")

    print_screen(term, "After erasing part of the copied box")
    return term


def main():
    """Run all rectangle operation demos"""
    print("=== VT420 Rectangle Operations Demo ===\n")
    print("Demonstrates DECFRA, DECCRA, and DECSERA sequences")

    demo_decfra()
    demo_deccra()
    demo_decsera()
    demo_combined()

    print("\n\n=== Summary ===")
    print("Rectangle operations provide efficient ways to:")
    print("  • DECFRA: Fill rectangular regions with characters")
    print("  • DECCRA: Copy rectangular regions to new locations")
    print("  • DECSERA: Erase rectangular regions")
    print("\nThese operations are used by advanced text editors like vim and emacs")
    print("for efficient screen manipulation.")


if __name__ == "__main__":
    main()
