#!/usr/bin/env python3
"""
Terminal Resize Example

This example demonstrates:
- Dynamically resizing the terminal
- SIGWINCH signal delivery to child process
- Terminal size environment variables
- Handling resize in running processes
"""

import time
from par_term_emu_core_rust import PtyTerminal


def test_basic_resize():
    """Test basic terminal resizing"""
    print("=== Basic Terminal Resize ===\n")

    term = PtyTerminal(80, 24)
    print(f"Initial size: {term.size()}")

    # Resize the terminal
    print("Resizing to 100x30...")
    term.resize(100, 30)
    print(f"New size: {term.size()}\n")

    # Verify scrollback is preserved
    term.spawn("/bin/echo", args=["Test", "message"])
    time.sleep(0.2)

    # Resize again
    print("Resizing to 120x40...")
    term.resize(120, 40)
    print(f"New size: {term.size()}")

    output = term.content()
    if "Test" in output or "message" in output:
        print("✓ Content preserved after resize\n")
    else:
        print("✗ Content may have been lost\n")

    term.wait()


def test_resize_with_shell():
    """Test resizing with a running shell"""
    print("=== Resize with Running Shell ===\n")

    term = PtyTerminal(80, 24)

    print(f"Starting shell at size: {term.size()}")
    term.spawn_shell()
    time.sleep(0.5)

    # Check current terminal size in shell
    print("\nChecking $COLUMNS and $LINES in shell...")
    term.write_str("echo COLUMNS=$COLUMNS LINES=$LINES\n")
    time.sleep(0.3)

    output = term.content()
    print("Output:")
    print(output[-200:])  # Last 200 chars

    # Resize the terminal
    print("\nResizing terminal to 100x30...")
    term.resize(100, 30)
    time.sleep(0.2)

    # The shell should receive SIGWINCH and update its size
    # Note: Some shells may need to be queried again to see the new size
    print("Checking size after resize...")
    term.write_str("echo COLUMNS=$COLUMNS LINES=$LINES\n")
    time.sleep(0.3)

    output = term.content()
    print("Output:")
    print(output[-200:])  # Last 200 chars

    # Clean up
    term.write_str("exit\n")
    time.sleep(0.2)

    if term.is_running():
        term.kill()

    print("\nNote: The shell receives SIGWINCH when the terminal is resized.")
    print("The shell can then update its internal state to match the new size.\n")


def test_resize_with_visual_program():
    """Test resizing with a program that uses terminal size"""
    print("=== Resize with Size-Aware Program ===\n")

    term = PtyTerminal(40, 10)

    print(f"Starting with small terminal: {term.size()}")

    # Create a simple script that prints the terminal size
    script = """
import sys
import os
cols = os.get_terminal_size().columns
lines = os.get_terminal_size().lines
print(f'Terminal size: {cols}x{lines}')
print('=' * min(cols, 40))
"""

    # Run the script
    term.spawn("/bin/sh", args=["-c", f"python3 -c '{script}'"])
    time.sleep(0.3)

    output = term.content()
    print("Output at 40x10:")
    print("-" * 50)
    print(output)
    print()

    term.wait()

    # Now try with a larger size
    term2 = PtyTerminal(80, 24)
    print(f"Starting with larger terminal: {term2.size()}")

    term2.spawn("/bin/sh", args=["-c", f"python3 -c '{script}'"])
    time.sleep(0.3)

    output = term2.content()
    print("Output at 80x24:")
    print("-" * 50)
    print(output)

    term2.wait()


def test_multiple_resizes():
    """Test multiple rapid resizes"""
    print("\n=== Multiple Rapid Resizes ===\n")

    term = PtyTerminal(80, 24)
    term.spawn_shell()
    time.sleep(0.5)

    sizes = [(60, 20), (100, 30), (80, 24), (120, 40), (70, 25)]

    print("Performing rapid resizes:")
    for cols, rows in sizes:
        print(f"  -> {cols}x{rows}")
        term.resize(cols, rows)
        time.sleep(0.1)

    print(f"\nFinal size: {term.size()}")

    # Verify shell is still responsive
    term.write_str("echo 'Still alive!'\n")
    time.sleep(0.3)

    output = term.content()
    if "Still alive" in output:
        print("✓ Shell survived multiple resizes\n")
    else:
        print("✗ Shell may have issues\n")

    term.write_str("exit\n")
    time.sleep(0.2)

    if term.is_running():
        term.kill()


def main():
    test_basic_resize()
    test_resize_with_shell()
    test_resize_with_visual_program()
    test_multiple_resizes()


if __name__ == "__main__":
    main()
