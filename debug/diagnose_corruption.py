#!/usr/bin/env python3
"""
Comprehensive diagnostic script for TUI corruption issues.

This script helps identify the source of corruption by:
1. Running textual in a terminal emulator
2. Capturing snapshots at regular intervals
3. Checking for corruption patterns in cell data
4. Logging detailed information about what's found
"""

from par_term_emu_core_rust import PtyTerminal
import time
import sys


def check_line_for_corruption(line_cells, line_num):
    """Check a line for corruption patterns."""
    text = "".join(char for char, _, _, _ in line_cells)
    issues = []

    # Check for escape characters that should never be in rendered cells
    if "\x1b" in text:
        issues.append(f"ESC char (0x1b) found at position {text.index(chr(0x1B))}")

    # Check for other control characters (except space/tab)
    for i, char in enumerate(text):
        if char and ord(char) < 32 and char not in [" ", "\t"]:
            issues.append(f"Control char 0x{ord(char):02x} at position {i}")

    # Check for SGR fragment patterns (semicolons + 'm')
    if text.count(";") >= 3 and "m" in text:
        # Find the suspicious section
        m_pos = text.index("m")
        start = max(0, m_pos - 20)
        end = min(len(text), m_pos + 10)
        snippet = text[start:end]
        issues.append(f"Possible SGR fragment: ...{repr(snippet)}...")

    # Check for CSI patterns
    if "CSI" in text or "[?" in text:
        issues.append("Possible escape sequence name in text")

    return text, issues


def main():
    print("Starting corruption diagnostic...")
    print("=" * 80)

    # Create terminal
    term = PtyTerminal(100, 40)
    term.spawn_shell()

    print(f"Terminal created: {term.size()}")
    print(f"Starting generation: {term.update_generation()}")
    print()

    # Wait for shell to be ready
    time.sleep(0.5)

    # Send command to run textual
    print("Launching textual...")
    term.write_str("uv run python -m textual\n")

    # Wait for textual to start
    time.sleep(3)

    print(f"Textual started. Generation: {term.update_generation()}")
    print()

    # Take multiple snapshots over time
    corruption_found = False
    for snapshot_num in range(5):
        print(
            f"--- Snapshot {snapshot_num + 1} (generation {term.update_generation()}) ---"
        )

        snapshot = term.create_snapshot()

        print(f"Screen: {'ALTERNATE' if snapshot.is_alt_screen else 'PRIMARY'}")
        print(f"Size: {snapshot.size}")
        print(f"Cursor: {snapshot.cursor_pos} (visible={snapshot.cursor_visible})")
        print()

        # Check first 10 lines for corruption
        for y in range(min(10, snapshot.size[1])):
            line_cells = snapshot.get_line(y)
            text, issues = check_line_for_corruption(line_cells, y)

            if issues:
                corruption_found = True
                print(f"⚠️  CORRUPTION DETECTED on line {y}:")
                print(f"   Text: {repr(text[:80])}")
                for issue in issues:
                    print(f"   - {issue}")

                # Show hex dump of first 40 chars
                hex_chars = " ".join(f"{ord(c):02x}" for c in text[:40])
                print(f"   Hex:  {hex_chars}")
                print()
            elif y < 3:  # Always show first 3 lines
                print(f"Line {y}: {repr(text[:80])}")

        print()

        if snapshot_num < 4:
            # Navigate in textual (send arrow keys)
            if snapshot_num % 2 == 0:
                term.write_str("\x1b[B")  # Down arrow
            else:
                term.write_str("\x1b[A")  # Up arrow
            time.sleep(1)

    # Final summary
    print("=" * 80)
    if corruption_found:
        print("❌ CORRUPTION WAS DETECTED")
        print("   Check the output above for details.")
    else:
        print("✅ NO CORRUPTION DETECTED")
        print("   All lines appear clean in snapshot data.")
    print()
    print(f"Final generation: {term.update_generation()}")

    # Clean up
    term.kill()

    return 1 if corruption_found else 0


if __name__ == "__main__":
    sys.exit(main())
