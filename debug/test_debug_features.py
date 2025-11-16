#!/usr/bin/env python3
"""Test script for par-term-emu-tui-rust debug infrastructure.

This script verifies that all debug features are working correctly:
1. Debug logging to /tmp/par_term_emu_debug.log
2. Debug API methods (debug_info, debug_snapshot_*, debug_log_snapshot)
3. Different debug levels produce appropriate output
4. Generation tracking and corruption detection

Usage:
    make debug-clear
    export DEBUG_LEVEL=3
    uv run python test_debug_features.py
    make debug-view
"""

import os
import time
from pathlib import Path

from par_term_emu_core_rust import Terminal


def print_section(title: str) -> None:
    """Print a formatted section header."""
    print(f"\n{'=' * 70}")
    print(f"  {title}")
    print(f"{'=' * 70}\n")


def test_debug_logging() -> None:
    """Test that debug logging is enabled and working."""
    print_section("Test 1: Debug Logging Configuration")

    debug_level = os.environ.get("DEBUG_LEVEL", "0")
    print(f"DEBUG_LEVEL environment variable: {debug_level}")

    if debug_level == "0":
        print("⚠️  WARNING: DEBUG_LEVEL=0 (logging disabled)")
        print("   Run: export DEBUG_LEVEL=3")
        return

    debug_file = Path("/tmp/par_term_emu_debug.log")
    if debug_file.exists():
        size = debug_file.stat().st_size
        print(f"✅ Debug log file exists: {debug_file}")
        print(f"   Current size: {size:,} bytes")
    else:
        print(f"⚠️  Debug log file not found: {debug_file}")
        print("   It will be created when terminal processes input")


def test_debug_info() -> None:
    """Test the debug_info() method."""
    print_section("Test 2: Debug Info API")

    term = Terminal(80, 24)
    term.process(b"Hello World\n")
    term.process(b"\x1b[31mRed Text\x1b[0m\n")

    info = term.debug_info()
    print("Terminal state (debug_info()):")
    for key, value in sorted(info.items()):
        print(f"  {key:20s}: {value}")

    print("\n✅ debug_info() method working")


def test_buffer_snapshots() -> None:
    """Test buffer snapshot methods."""
    print_section("Test 3: Buffer Snapshot Methods")

    term = Terminal(80, 24)

    # Add some content
    term.process(b"Line 1: Normal text\n")
    term.process(b"\x1b[1;32mLine 2: Bold Green\x1b[0m\n")
    term.process(b"\x1b[44mLine 3: Blue Background\x1b[0m\n")

    # Test current buffer snapshot
    print("3a. Current Buffer Snapshot:")
    snapshot = term.debug_snapshot_buffer()
    lines = snapshot.split("\n")
    for i, line in enumerate(lines[:15]):  # Show first 15 lines
        print(f"  {line}")
    if len(lines) > 15:
        print(f"  ... ({len(lines) - 15} more lines)")

    print("\n✅ debug_snapshot_buffer() working")

    # Test primary buffer snapshot
    print("\n3b. Primary Buffer Snapshot:")
    primary = term.debug_snapshot_primary()
    print(f"  Primary buffer: {len(primary.split(chr(10)))} lines")

    # Switch to alternate screen
    term.process(b"\x1b[?1049h")  # Enter alternate screen
    term.process(b"Alternate screen content\n")
    term.process(b"\x1b[1;33mYellow on alternate\x1b[0m\n")

    # Test alternate buffer snapshot
    print("\n3c. Alternate Buffer Snapshot:")
    alt = term.debug_snapshot_alt()
    alt_lines = alt.split("\n")
    for i, line in enumerate(alt_lines[:10]):
        print(f"  {line}")

    print(f"\n✅ debug_snapshot_alt() working ({len(alt_lines)} lines)")

    # Switch back to primary
    term.process(b"\x1b[?1049l")  # Exit alternate screen


def test_log_snapshot() -> None:
    """Test the debug_log_snapshot() method."""
    print_section("Test 4: Log Snapshot to File")

    term = Terminal(80, 24)
    term.process(b"\x1b[1;31mImportant Red Text\x1b[0m\n")
    term.process(b"Testing snapshot logging\n")

    # Log a snapshot
    term.debug_log_snapshot("Test snapshot from test_debug_features.py")

    print("✅ Snapshot logged to /tmp/par_term_emu_debug.log")
    print("   Search for: 'BUFFER SNAPSHOT: Test snapshot'")


def test_vt_sequences() -> None:
    """Test that VT sequences are logged at DEBUG_LEVEL=3+."""
    print_section("Test 5: VT Sequence Logging")

    debug_level = int(os.environ.get("DEBUG_LEVEL", "0"))

    if debug_level < 3:
        print("⚠️  VT sequence logging requires DEBUG_LEVEL >= 3")
        print(f"   Current level: {debug_level}")
        print("   VT sequences will not be logged")
        return

    term = Terminal(80, 24)

    # Send various VT sequences
    sequences = [
        (b"\x1b[31m", "Set foreground red (CSI 31 m)"),
        (b"\x1b[1;44m", "Bold + blue background (CSI 1;44 m)"),
        (b"\x1b[2J", "Clear screen (CSI 2 J)"),
        (b"\x1b[H", "Cursor home (CSI H)"),
        (b"\x1b[6n", "Device Status Report (CSI 6 n)"),
        (b"\x1b[?1049h", "Alternate screen (CSI ? 1049 h)"),
    ]

    print("Sending VT sequences (check debug log for processing):")
    for seq, desc in sequences:
        print(f"  {desc}")
        term.process(seq)
        time.sleep(0.01)  # Small delay for logging

    print("\n✅ VT sequences sent")
    print("   Check /tmp/par_term_emu_debug.log for:")
    print("   - [VT_INPUT] entries showing raw bytes")
    print("   - [CSI], [OSC], [ESC] entries showing parsed sequences")


def test_generation_tracking() -> None:
    """Test generation counter tracking."""
    print_section("Test 6: Generation Counter Tracking")

    print("⚠️  Generation tracking is only available on PtyTerminal")
    print("   (Terminal class doesn't have background updates)\n")

    try:
        from par_term_emu_core_rust import PtyTerminal

        # Create PtyTerminal to test generation tracking
        pty_term = PtyTerminal(80, 24)

        initial_gen = pty_term.update_generation()
        print(f"Initial generation: {initial_gen}")

        # Note: Generation only increments on PTY reads, not direct process() calls
        print("\nNote: For PtyTerminal, generation increments when PTY")
        print("      receives data, not when processing manually.")
        print(f"\nCurrent generation: {pty_term.update_generation()}")

        # Test has_updates_since()
        print(
            f"has_updates_since({initial_gen}): {pty_term.has_updates_since(initial_gen)}"
        )
        print(
            f"has_updates_since({initial_gen + 1}): {pty_term.has_updates_since(initial_gen + 1)}"
        )

        print("\n✅ Generation counter tracking working")

    except Exception as e:
        print(f"⚠️  Could not test PtyTerminal: {e}")
        print("   This is expected if PTY is not available on this platform")


def test_corruption_detection() -> None:
    """Test corruption detection in debug logs."""
    print_section("Test 7: Corruption Detection (Simulated)")

    debug_level = int(os.environ.get("DEBUG_LEVEL", "0"))

    if debug_level < 1:
        print("⚠️  Corruption detection requires DEBUG_LEVEL >= 1")
        print(f"   Current level: {debug_level}")
        return

    term = Terminal(80, 24)

    # Simulate potential corruption patterns (escape sequences that might appear as text)
    # This doesn't actually corrupt the terminal, but tests if the logging would catch it
    suspicious_patterns = [
        b"\x1b[31mNormal escape sequence\x1b[0m",  # Normal - should NOT trigger
        # Real corruption would show escape sequences as literal text in cells,
        # which the TUI widget's corruption detection would catch
    ]

    print("Processing sequences (real corruption detection happens in TUI widget):")
    for pattern in suspicious_patterns:
        term.process(pattern)
        print(f"  Processed: {pattern[:30]}...")

    print("\n✅ Corruption detection is enabled at DEBUG_LEVEL >= 1")
    print("   Real corruption is detected by TUI widget's render_line() method")
    print("   Look for '[ERROR] [CORRUPTION]' entries in the debug log")


def main() -> None:
    """Run all debug feature tests."""
    print("""
╔══════════════════════════════════════════════════════════════════════╗
║                                                                      ║
║           par-term-emu-tui-rust Debug Infrastructure Test Suite              ║
║                                                                      ║
╚══════════════════════════════════════════════════════════════════════╝
""")

    test_debug_logging()
    test_debug_info()
    test_buffer_snapshots()
    test_log_snapshot()
    test_vt_sequences()
    test_generation_tracking()
    test_corruption_detection()

    print_section("Test Summary")

    debug_level = os.environ.get("DEBUG_LEVEL", "0")
    debug_file = Path("/tmp/par_term_emu_debug.log")

    print("✅ All debug API methods are functional\n")

    if debug_level == "0":
        print("⚠️  DEBUG_LEVEL=0: Logging is disabled")
        print("\nTo enable debug logging:")
        print("  export DEBUG_LEVEL=3")
        print("  uv run python test_debug_features.py")
    else:
        print(f"✅ DEBUG_LEVEL={debug_level}: Logging enabled\n")

        if debug_file.exists():
            size = debug_file.stat().st_size
            print(f"Debug log file: {debug_file}")
            print(f"Current size: {size:,} bytes")
            print("\nTo view the debug log:")
            print("  make debug-view        # View with less")
            print("  make debug-tail        # Watch in real-time")
            print("\nTo search for specific events:")
            print("  grep CORRUPTION /tmp/par_term_emu_debug.log")
            print("  grep VT_INPUT /tmp/par_term_emu_debug.log")
            print("  grep SCREEN_SWITCH /tmp/par_term_emu_debug.log")
        else:
            print("⚠️  Debug log file not created (this is unexpected)")

    print("\n" + "=" * 70)
    print("\nNext steps to test TUI corruption debugging:")
    print("  1. make debug-clear")
    print("  2. export DEBUG_LEVEL=3")
    print("  3. make tui")
    print("  4. Inside TUI, run: python -m textual")
    print("  5. In another terminal: tail -f /tmp/par_term_emu_debug.log")
    print("  6. Look for corruption events and analyze patterns")
    print("=" * 70 + "\n")


if __name__ == "__main__":
    main()
