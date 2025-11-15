#!/usr/bin/env python3
"""
Test script to verify wide character rendering.
This script demonstrates that wide characters (emoji, CJK) are properly handled.
"""

from par_term_emu_core_rust import Terminal


def test_basic_rendering():
    """Test basic wide character rendering"""
    print("=" * 60)
    print("Testing Wide Character Support")
    print("=" * 60)
    print()

    term = Terminal(80, 10)

    # Test 1: CJK characters
    print("Test 1: CJK Characters")
    term.process_str("Hello 中文 World")
    content = term.content()
    print(f"Content: {repr(content[:30])}")

    # Check cursor position
    col, row = term.cursor_position()
    print(f"Cursor position: col={col}, row={row}")
    print(
        "Expected: col=16 (Hello(5) + space(1) + 中(2) + 文(2) + space(1) + World(5))"
    )
    assert col == 16, f"Cursor at wrong position: {col} instead of 16"
    print("✓ Cursor position correct")
    print()

    # Test 2: Check flags
    print("Test 2: Wide Character Flags")
    attrs_wide = term.get_attributes(6, 0)  # '中' at position 6
    attrs_spacer = term.get_attributes(7, 0)  # Spacer at position 7

    print(
        f"Character at col 6: wide_char={attrs_wide.wide_char}, wide_char_spacer={attrs_wide.wide_char_spacer}"
    )
    print(
        f"Character at col 7: wide_char={attrs_spacer.wide_char}, wide_char_spacer={attrs_spacer.wide_char_spacer}"
    )

    assert attrs_wide.wide_char, "Wide char flag not set"
    assert not attrs_wide.wide_char_spacer, "Spacer flag incorrectly set on wide char"
    assert not attrs_spacer.wide_char, "Wide char flag incorrectly set on spacer"
    assert attrs_spacer.wide_char_spacer, "Spacer flag not set"
    print("✓ Flags correctly set")
    print()

    # Test 3: Emoji
    print("Test 3: Emoji")
    term.reset()
    term.process_str("Hello 🎉😀🚀 World")
    col, row = term.cursor_position()
    print(f"Content: {repr(term.content()[:30])}")
    print(f"Cursor position: col={col}, row={row}")
    # Hello(5) + space(1) + 🎉(2) + 😀(2) + 🚀(2) + space(1) + World(5) = 18
    print("Expected: col=18")
    assert col == 18, f"Cursor at wrong position: {col} instead of 18"
    print("✓ Emoji rendering correct")
    print()

    # Test 4: Mixed content
    print("Test 4: Mixed ASCII and Wide Characters")
    term.reset()
    term.process_str("A中B文C")
    col, row = term.cursor_position()
    print(f"Content: {repr(term.content()[:20])}")
    print(f"Cursor position: col={col}, row={row}")
    # A(1) + 中(2) + B(1) + 文(2) + C(1) = 7
    print("Expected: col=7")
    assert col == 7, f"Cursor at wrong position: {col} instead of 7"

    # Check all flags
    flags = []
    for i in range(7):
        attrs = term.get_attributes(i, 0)
        flags.append((i, attrs.wide_char, attrs.wide_char_spacer))

    print("Position flags:")
    for i, wc, sp in flags:
        char = term.get_char(i, 0) or " "
        print(f"  col {i}: '{char}' wide={wc} spacer={sp}")

    print("✓ Mixed content correct")
    print()

    # Test 5: Snapshot preserves flags
    print("Test 5: Snapshot Preserves Flags")
    term.reset()
    term.process_str("Test 中文")
    snapshot = term.create_snapshot()
    line = snapshot.get_line(0)

    char5, _, _, attrs5 = line[5]
    char6, _, _, attrs6 = line[6]

    print(
        f"Position 5: '{char5}' wide={attrs5.wide_char} spacer={attrs5.wide_char_spacer}"
    )
    print(
        f"Position 6: '{char6}' wide={attrs6.wide_char} spacer={attrs6.wide_char_spacer}"
    )

    assert char5 == "中"
    assert attrs5.wide_char
    assert not attrs5.wide_char_spacer
    assert attrs6.wide_char_spacer
    print("✓ Snapshot preserves flags correctly")
    print()

    print("=" * 60)
    print("All Tests PASSED! ✓")
    print("=" * 60)
    print()
    print("Wide character support is working correctly!")
    print("Wide characters now properly occupy 2 columns and spacer cells are marked.")
    print()
    print("Next step: Test in TUI with: make tui")
    print("Then run: uv run python -m textual")
    print("Box drawing and emoji should now render correctly aligned.")


if __name__ == "__main__":
    test_basic_rendering()
