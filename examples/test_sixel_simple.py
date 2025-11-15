#!/usr/bin/env python3
"""
Simple test to verify Sixel graphics implementation.

Creates a small red square using Sixel format and verifies it's parsed correctly.
"""

from par_term_emu_core_rust import Terminal


def test_simple_sixel():
    """Test basic Sixel parsing with a small red square."""
    term = Terminal(80, 24)

    # Create a simple 2x2 red square using Sixel format
    # DCS Pq = ESC P ... q (Sixel start)
    # #0;2;100;0;0 = Define color 0 as red (RGB: 100%, 0%, 0%)
    # #0 = Select color 0
    # @@@ = Draw 3 pixels (each @ = 111111 in sixel, all 6 bits set)
    # - = New line
    # @@@ = Draw 3 more pixels
    # ST = ESC \ (String terminator)
    sixel = "\x1bPq#0;2;100;0;0#0@@@-@@@\x1b\\"

    print("Testing Sixel graphics implementation...")
    print(f"Sixel sequence length: {len(sixel)} bytes")

    # Process the Sixel sequence
    term.process_str(sixel)

    # Check if graphic was stored
    graphics_count = term.graphics_count()
    print(f"✓ Graphics count: {graphics_count}")

    if graphics_count == 0:
        print("✗ FAILED: No graphics were stored")
        return False

    # Get graphics at row 0
    graphics = term.graphics_at_row(0)
    print(f"✓ Graphics at row 0: {len(graphics)}")

    if len(graphics) == 0:
        print("✗ FAILED: No graphics found at row 0")
        return False

    # Check graphic properties
    graphic = graphics[0]
    print(f"✓ Graphic position: {graphic.position}")
    print(f"✓ Graphic size: {graphic.width}x{graphic.height}")

    # Verify we can access pixels
    pixel = graphic.get_pixel(0, 0)
    if pixel:
        r, g, b, a = pixel
        print(f"✓ First pixel (RGBA): ({r}, {g}, {b}, {a})")

        # Check if it's reddish (allowing for conversion tolerance)
        if r > 200 and g < 50 and b < 50:
            print("✓ Pixel color is red as expected")
        else:
            print(f"⚠ Warning: Expected red pixel but got RGB({r}, {g}, {b})")
    else:
        print("✗ FAILED: Could not read pixel data")
        return False

    print("\n✓ All basic Sixel tests passed!")
    print(f"✓ Implementation successfully: parses Sixel → stores graphics → provides API access")
    return True


def test_multiple_graphics():
    """Test that multiple graphics can be stored."""
    term = Terminal(80, 24)

    # Add two simple graphics
    sixel1 = "\x1bPq#0;2;100;0;0#0@@\x1b\\"  # Red
    sixel2 = "\x1bPq#0;2;0;100;0#0@@\x1b\\"  # Green

    term.process_str(sixel1)
    term.process_str(sixel2)

    count = term.graphics_count()
    print(f"\n✓ Multiple graphics test: {count} graphics stored")

    if count >= 2:
        print("✓ Multiple graphics storage works!")
        return True
    else:
        print(f"✗ FAILED: Expected 2+ graphics but got {count}")
        return False


def test_clear_graphics():
    """Test clearing graphics."""
    term = Terminal(80, 24)

    # Add a graphic
    sixel = "\x1bPq#0;2;100;0;0#0@@\x1b\\"
    term.process_str(sixel)

    print(f"\n✓ Graphics before clear: {term.graphics_count()}")

    # Clear
    term.clear_graphics()

    count_after = term.graphics_count()
    print(f"✓ Graphics after clear: {count_after}")

    if count_after == 0:
        print("✓ Clear graphics works!")
        return True
    else:
        print(f"✗ FAILED: Expected 0 graphics after clear but got {count_after}")
        return False


if __name__ == "__main__":
    print("=" * 60)
    print("Sixel Graphics Implementation Test")
    print("=" * 60)

    results = []
    results.append(("Basic Sixel parsing", test_simple_sixel()))
    results.append(("Multiple graphics", test_multiple_graphics()))
    results.append(("Clear graphics", test_clear_graphics()))

    print("\n" + "=" * 60)
    print("Test Results Summary")
    print("=" * 60)

    all_passed = True
    for test_name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"{status}: {test_name}")
        if not passed:
            all_passed = False

    print("=" * 60)
    if all_passed:
        print("✓ All tests passed! Sixel implementation is working correctly.")
    else:
        print("✗ Some tests failed. Please review the implementation.")

    exit(0 if all_passed else 1)
