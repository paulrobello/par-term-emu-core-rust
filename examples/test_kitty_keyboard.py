#!/usr/bin/env python3
"""
Test Kitty Keyboard Protocol implementation.

Demonstrates the Python API for Kitty Keyboard Protocol flags.
"""

from par_term_emu_core_rust import Terminal


def test_keyboard_flags():
    """Test Kitty Keyboard Protocol flags API."""
    term = Terminal(80, 24)

    print("=" * 60)
    print("Kitty Keyboard Protocol Test")
    print("=" * 60)

    # Test 1: Initial state
    flags = term.keyboard_flags()
    print(f"✓ Initial keyboard flags: {flags}")
    assert flags == 0, f"Expected 0, got {flags}"

    # Test 2: Set flags
    print("\n--- Setting flags to 1 (disambiguate) ---")
    term.set_keyboard_flags(1, mode=1)
    flags = term.keyboard_flags()
    print(f"✓ After set_keyboard_flags(1): {flags}")
    assert flags == 1, f"Expected 1, got {flags}"

    # Test 3: Query flags
    print("\n--- Querying flags ---")
    term.query_keyboard_flags()
    response = term.drain_responses()
    print(f"✓ Query response: {response!r}")
    assert response == b"\x1b[?1u", f"Expected CSI ?1u, got {response!r}"

    # Test 4: Push flags
    print("\n--- Pushing flags ---")
    term.push_keyboard_flags(3)  # Push 3 (disambiguate + report events)
    flags = term.keyboard_flags()
    print(f"✓ After push_keyboard_flags(3): {flags}")
    assert flags == 3, f"Expected 3, got {flags}"

    # Test 5: Pop flags
    print("\n--- Popping flags ---")
    term.pop_keyboard_flags(1)
    flags = term.keyboard_flags()
    print(f"✓ After pop_keyboard_flags(1): {flags}")
    assert flags == 1, f"Expected 1 (restored from stack), got {flags}"

    # Test 6: Disable all
    print("\n--- Disabling all flags ---")
    term.set_keyboard_flags(0, mode=0)
    flags = term.keyboard_flags()
    print(f"✓ After disable (mode=0): {flags}")
    assert flags == 0, f"Expected 0, got {flags}"

    # Test 7: Multiple flags
    print("\n--- Setting multiple flags ---")
    # 1 (disambiguate) + 2 (report events) + 8 (report all) = 11
    term.set_keyboard_flags(11, mode=1)
    flags = term.keyboard_flags()
    print(f"✓ After set_keyboard_flags(11): {flags}")
    assert flags == 11, f"Expected 11, got {flags}"

    print("\n" + "=" * 60)
    print("✓ All Kitty Keyboard Protocol tests passed!")
    print("=" * 60)

    print("\nKitty Keyboard Protocol Flags:")
    print("  1  = Disambiguate escape codes")
    print("  2  = Report event types")
    print("  4  = Report alternate key values")
    print("  8  = Report all keys as escape codes")
    print("  16 = Report associated text")
    return True


if __name__ == "__main__":
    try:
        test_keyboard_flags()
        exit(0)
    except Exception as e:
        print(f"\n✗ Test failed: {e}")
        import traceback
        traceback.print_exc()
        exit(1)
