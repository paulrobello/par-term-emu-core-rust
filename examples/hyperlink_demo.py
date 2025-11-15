#!/usr/bin/env python3
"""Demonstrate hyperlink support in par-term-emu.

This script creates a terminal with various hyperlinks to test:
1. Basic hyperlinks
2. Hyperlinks with styled text (bold, colors)
3. Multiple hyperlinks on the same line
4. Hyperlinks spanning multiple characters
"""

import time
from par_term_emu_core_rust import Terminal

# Create a terminal
term = Terminal(80, 24)

# Test 1: Basic hyperlink
term.process_str("Test 1: Basic hyperlink\r\n")
term.process_str("\x1b]8;;https://github.com/paulrobello/par-term-emu-rust\x1b\\")
term.process_str("Click here for the repository")
term.process_str("\x1b]8;;\x1b\\")
term.process_str("\r\n\r\n")

# Test 2: Hyperlink with styled text (bold and red)
term.process_str("Test 2: Styled hyperlink\r\n")
term.process_str("\x1b]8;;https://www.rust-lang.org\x1b\\")
term.process_str("\x1b[1;31mBold red link to Rust\x1b[0m")
term.process_str("\x1b]8;;\x1b\\")
term.process_str("\r\n\r\n")

# Test 3: Multiple hyperlinks on same line
term.process_str("Test 3: Multiple links\r\n")
term.process_str("Visit ")
term.process_str("\x1b]8;;https://www.python.org\x1b\\Python\x1b]8;;\x1b\\")
term.process_str(" or ")
term.process_str("\x1b]8;;https://www.rust-lang.org\x1b\\Rust\x1b]8;;\x1b\\")
term.process_str(" websites")
term.process_str("\r\n\r\n")

# Test 4: Hyperlink with different colors
term.process_str("Test 4: Colored links\r\n")
term.process_str("\x1b]8;;https://github.com\x1b\\")
term.process_str("\x1b[34mBlue GitHub link\x1b[0m")
term.process_str("\x1b]8;;\x1b\\")
term.process_str(" and ")
term.process_str("\x1b]8;;https://docs.rs\x1b\\")
term.process_str("\x1b[32mGreen docs.rs link\x1b[0m")
term.process_str("\x1b]8;;\x1b\\")
term.process_str("\r\n\r\n")

# Test 5: Hyperlink with underline
term.process_str("Test 5: Underlined link\r\n")
term.process_str("\x1b]8;;https://pyo3.rs\x1b\\")
term.process_str("\x1b[4mUnderlined PyO3 link\x1b[0m")
term.process_str("\x1b]8;;\x1b\\")
term.process_str("\r\n\r\n")

# Print the terminal content
cols, rows = term.size()
print("=" * 80)
print("Terminal content with hyperlinks:")
print("=" * 80)
for row in range(rows):
    line = term.get_line(row)
    print(line)

print("\n" + "=" * 80)
print("Hyperlink details:")
print("=" * 80)

# Show hyperlink details
for row in range(rows):
    for col in range(cols):
        url = term.get_hyperlink(col, row)
        if url:
            char = term.get_char(col, row)
            print(f"Row {row}, Col {col}: '{char}' -> {url}")

print("\n" + "=" * 80)
print("To test in TUI:")
print("=" * 80)
print("1. Run: make tui")
print("2. In the shell, run:")
print("   echo -e '\\x1b]8;;https://example.com\\x1b\\\\Click me\\x1b]8;;\\x1b\\\\'")
print("3. If your terminal supports OSC 8, the text should be clickable")
print("   Supported terminals: iTerm2, VS Code, Kitty, WezTerm, Windows Terminal")
print("=" * 80)
