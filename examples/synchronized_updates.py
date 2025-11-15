#!/usr/bin/env python3
"""
Example demonstrating synchronized updates (DEC 2026) feature.

Synchronized updates allow applications to batch multiple screen updates
and apply them atomically, reducing flicker in terminal applications.

Usage:
    python examples/synchronized_updates.py
"""

from par_term_emu_core_rust import Terminal


def main():
    """Demonstrate synchronized updates feature"""
    term = Terminal(80, 24)

    print("=== Synchronized Updates (DEC 2026) Demo ===\n")

    # Check initial state
    print(f"1. Initial state - synchronized updates: {term.synchronized_updates()}")
    assert not term.synchronized_updates()

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")
    print(f"2. After CSI ? 2026 h - synchronized updates: {term.synchronized_updates()}")
    assert term.synchronized_updates()

    # Send updates while synchronized mode is on
    print("\n3. Sending updates while synchronized mode is ON...")
    term.process_str("Line 1: This is buffered\r\n")
    term.process_str("Line 2: This is also buffered\r\n")
    term.process_str("\x1b[31mLine 3: Red text, buffered too\x1b[0m\r\n")

    content = term.content()
    print(f"   Terminal content (should be empty): '{content[:50]}...'")
    assert "Line 1" not in content
    assert "Line 2" not in content
    assert "Line 3" not in content

    # Disable synchronized updates - this flushes the buffer
    print("\n4. Disabling synchronized updates (CSI ? 2026 l) - flushes buffer...")
    term.process_str("\x1b[?2026l")
    assert not term.synchronized_updates()

    content = term.content()
    print(f"   Terminal content (should show all lines):")
    for i, line in enumerate(content.split("\n")[:5], 1):
        print(f"      Line {i}: {line.strip()}")

    assert "Line 1" in content
    assert "Line 2" in content
    assert "Line 3" in content

    # Demonstrate manual flush
    print("\n5. Demonstrating manual flush...")
    term.process_str("\x1b[2J\x1b[H")  # Clear screen and home cursor
    term.process_str("\x1b[?2026h")  # Enable synchronized updates again

    term.process_str("Before manual flush")
    print(f"   Content before manual flush: '{term.content().strip()}'")
    assert "Before manual flush" not in term.content()

    term.flush_synchronized_updates()
    print(f"   Content after manual flush: '{term.content().strip()}'")
    assert "Before manual flush" in term.content()
    assert term.synchronized_updates()  # Mode still enabled

    # More content gets buffered
    term.process_str("\r\nAfter manual flush")
    print(f"   Added more text (buffered): '{term.content().strip()}'")
    assert "After manual flush" not in term.content()

    # Final disable to flush
    term.process_str("\x1b[?2026l")
    print(f"   Final content: '{term.content().strip()}'")
    assert "After manual flush" in term.content()

    # Use case example: Animated progress bar
    print("\n6. Use case: Animated progress bar without flicker...")
    term.process_str("\x1b[2J\x1b[H")  # Clear screen

    for i in range(0, 101, 10):
        # Enable synchronized updates for this frame
        term.process_str("\x1b[?2026h")
        term.process_str("\x1b[H")  # Home cursor
        term.process_str(f"Progress: [{('=' * (i // 10)).ljust(10)}] {i}%")
        term.process_str("\x1b[?2026l")  # Flush this frame

    print(f"   Final progress bar: {term.content().strip()}")

    print("\n=== All tests passed! ===")


if __name__ == "__main__":
    main()
