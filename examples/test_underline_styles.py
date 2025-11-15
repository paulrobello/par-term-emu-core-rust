#!/usr/bin/env python3
"""
Test underline styles (SGR 4:x).

Demonstrates the modern underline style support for better text decoration.
Used by modern terminals for syntax highlighting, spell check, error indicators, etc.
"""

from par_term_emu_core_rust import Terminal, UnderlineStyle


def test_basic_underline():
    """Test basic underline (SGR 4)."""
    print("=" * 60)
    print("Basic Underline Test (SGR 4)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Basic underline defaults to straight
    term.process_str("\x1b[4mBasic underline")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Straight)
    print("✓ SGR 4 sets straight underline by default")

    return True


def test_straight_underline():
    """Test explicit straight underline (SGR 4:1)."""
    print("\n" + "=" * 60)
    print("Straight Underline Test (SGR 4:1)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Explicit straight underline
    term.process_str("\x1b[4:1mStraight underline")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Straight)
    print("✓ SGR 4:1 sets straight underline")

    return True


def test_double_underline():
    """Test double underline (SGR 4:2)."""
    print("\n" + "=" * 60)
    print("Double Underline Test (SGR 4:2)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Double underline
    term.process_str("\x1b[4:2mDouble underline")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Double)
    print("✓ SGR 4:2 sets double underline")
    print("  Use case: Emphasis, important text")

    return True


def test_curly_underline():
    """Test curly underline (SGR 4:3) - for errors/spell check."""
    print("\n" + "=" * 60)
    print("Curly Underline Test (SGR 4:3)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Curly underline (error indicator)
    term.process_str("\x1b[4:3mSpelling eror")  # Intentional typo

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Curly)
    print("✓ SGR 4:3 sets curly underline")
    print("  Use case: Spell check errors, LSP diagnostics")
    print("  Common in: VSCode, Neovim, modern IDEs")

    return True


def test_dotted_underline():
    """Test dotted underline (SGR 4:4)."""
    print("\n" + "=" * 60)
    print("Dotted Underline Test (SGR 4:4)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Dotted underline
    term.process_str("\x1b[4:4mDotted underline")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Dotted)
    print("✓ SGR 4:4 sets dotted underline")
    print("  Use case: Subtle emphasis, suggestions")

    return True


def test_dashed_underline():
    """Test dashed underline (SGR 4:5)."""
    print("\n" + "=" * 60)
    print("Dashed Underline Test (SGR 4:5)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Dashed underline
    term.process_str("\x1b[4:5mDashed underline")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Dashed)
    print("✓ SGR 4:5 sets dashed underline")
    print("  Use case: Links, temporary markers")

    return True


def test_disable_underline():
    """Test disabling underline (SGR 24)."""
    print("\n" + "=" * 60)
    print("Disable Underline Test (SGR 24)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Set underline then disable
    term.process_str("\x1b[4:3mUnderlined")
    term.process_str("\x1b[24m Normal")

    # Check underlined text
    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.underline is True
    assert int(attrs.underline_style) == int(UnderlineStyle.Curly)
    print("✓ Text before SGR 24 has curly underline")

    # Check normal text
    attrs = term.get_attributes(10, 0)
    assert attrs is not None
    assert attrs.underline is False
    assert int(attrs.underline_style) == int(0)
    print("✓ Text after SGR 24 has no underline")

    return True


def test_reset_underline():
    """Test resetting all attributes (SGR 0)."""
    print("\n" + "=" * 60)
    print("Reset All Attributes Test (SGR 0)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Set underline then reset all
    term.process_str("\x1b[4:2mDouble")
    term.process_str("\x1b[0m Normal")

    # Check underlined text
    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert int(attrs.underline_style) == int(UnderlineStyle.Double)
    print("✓ Text before SGR 0 has double underline")

    # Check reset text
    attrs = term.get_attributes(6, 0)
    assert attrs is not None
    assert attrs.underline is False
    assert int(attrs.underline_style) == int(0)
    print("✓ Text after SGR 0 has no underline")

    return True


def test_style_switching():
    """Test switching between different underline styles."""
    print("\n" + "=" * 60)
    print("Style Switching Test")
    print("=" * 60)

    term = Terminal(80, 24)

    # Write text with different underline styles
    term.process_str("\x1b[4:1mStraight \x1b[4:3mCurly \x1b[4:2mDouble \x1b[24mNone")

    # Check each section
    attrs = term.get_attributes(0, 0)
    assert int(attrs.underline_style) == int(UnderlineStyle.Straight)
    print("✓ Position 0: Straight")

    attrs = term.get_attributes(9, 0)
    assert int(attrs.underline_style) == int(UnderlineStyle.Curly)
    print("✓ Position 9: Curly")

    attrs = term.get_attributes(15, 0)
    assert int(attrs.underline_style) == int(UnderlineStyle.Double)
    print("✓ Position 15: Double")

    attrs = term.get_attributes(22, 0)
    assert int(attrs.underline_style) == int(0)
    print("✓ Position 22: None")
    print("✓ Successfully switched between all underline styles")

    return True


def test_lsp_integration_demo():
    """Demo: LSP-style error/warning indicators."""
    print("\n" + "=" * 60)
    print("LSP Integration Demo")
    print("=" * 60)

    term = Terminal(80, 24)

    # Simulate LSP error (red curly underline)
    term.process_str("let \x1b[31;4:3mvariabel\x1b[0m = 42;")  # Typo with red curly

    attrs = term.get_attributes(4, 0)
    assert int(attrs.underline_style) == int(UnderlineStyle.Curly)
    print("✓ LSP error indicator: red + curly underline")
    print("  Used by: Neovim LSP, VSCode, Helix, etc.")

    return True


if __name__ == "__main__":
    print("\n" + "=" * 70)
    print(" " * 20 + "UNDERLINE STYLES TEST SUITE")
    print("=" * 70)

    results = []
    results.append(("Basic Underline", test_basic_underline()))
    results.append(("Straight Underline", test_straight_underline()))
    results.append(("Double Underline", test_double_underline()))
    results.append(("Curly Underline", test_curly_underline()))
    results.append(("Dotted Underline", test_dotted_underline()))
    results.append(("Dashed Underline", test_dashed_underline()))
    results.append(("Disable Underline", test_disable_underline()))
    results.append(("Reset Underline", test_reset_underline()))
    results.append(("Style Switching", test_style_switching()))
    results.append(("LSP Integration", test_lsp_integration_demo()))

    print("\n" + "=" * 70)
    print("TEST RESULTS SUMMARY")
    print("=" * 70)

    all_passed = True
    for test_name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"{status}: {test_name}")
        if not passed:
            all_passed = False

    print("=" * 70)
    if all_passed:
        print("✓ All tests passed! Underline styles working correctly.")
        print("\nSupported Styles (SGR 4:x):")
        print("  0 - None (disable)")
        print("  1 - Straight (default, also just SGR 4)")
        print("  2 - Double")
        print("  3 - Curly (errors, spell check)")
        print("  4 - Dotted")
        print("  5 - Dashed")
        print("\nCommon Use Cases:")
        print("  - LSP error/warning indicators (curly)")
        print("  - Spell check markers (curly)")
        print("  - Link underlines (dashed)")
        print("  - Emphasis variations (double, dotted)")
        print("\nCompatibility:")
        print("  - Kitty, iTerm2, Alacritty, WezTerm")
        print("  - VSCode terminal, Neovim, Helix")
    else:
        print("✗ Some tests failed. Please review the implementation.")

    print("=" * 70)
    exit(0 if all_passed else 1)
