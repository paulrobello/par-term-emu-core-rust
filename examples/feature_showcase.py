#!/usr/bin/env python3
"""
Feature Showcase for par_term_emu

This interactive example demonstrates the major features of the terminal emulator:
- VT100/VT220/VT320/VT420 compatibility
- Cursor styles and colors
- Mouse tracking
- Sixel graphics
- Scrolling regions
- Clipboard operations (OSC 52)
- Kitty keyboard protocol
- And more!

Usage:
    python3 examples/feature_showcase.py
"""

import sys
import time
from par_term_emu_core_rust import Terminal, CursorStyle, UnderlineStyle


def print_header(title: str):
    """Print a formatted section header."""
    print("\n" + "=" * 70)
    print(f"  {title}")
    print("=" * 70 + "\n")


def demonstrate_colors(term: Terminal):
    """Demonstrate color support."""
    print_header("Color Support (256 colors + true color)")

    # 16 basic colors
    print("Basic 16 colors:")
    for i in range(16):
        term.process_str(f"\x1b[48;5;{i}m  ")
    term.process_str("\x1b[0m\n")

    # 256 color palette
    print("\n256-color palette (showing sample):")
    for row in range(6):
        for col in range(36):
            color = 16 + row * 36 + col
            term.process_str(f"\x1b[48;5;{color}m ")
        term.process_str("\x1b[0m\n")

    # True color (RGB)
    print("\nTrue color RGB gradient:")
    for i in range(70):
        r = int(255 * (i / 70))
        g = int(128 * (1 - i / 70))
        b = int(192 * (0.5 + 0.5 * (i / 70)))
        term.process_str(f"\x1b[38;2;{r};{g};{b}m█")
    term.process_str("\x1b[0m\n")

    # Display current content
    print(term.content())


def demonstrate_text_attributes(term: Terminal):
    """Demonstrate text attributes and styles."""
    print_header("Text Attributes & Underline Styles")

    term.reset()

    # Basic SGR attributes
    attributes = [
        ("\x1b[1mBold\x1b[0m", "Bold text"),
        ("\x1b[2mDim\x1b[0m", "Dim/faint text"),
        ("\x1b[3mItalic\x1b[0m", "Italic text"),
        ("\x1b[4mUnderline\x1b[0m", "Underlined text"),
        ("\x1b[5mBlink\x1b[0m", "Blinking text"),
        ("\x1b[7mReverse\x1b[0m", "Reverse video"),
        ("\x1b[9mStrikethrough\x1b[0m", "Strikethrough text"),
    ]

    print("Basic text attributes:")
    for seq, desc in attributes:
        term.process_str(f"{seq} - {desc}\n")

    # Underline styles
    print("\nUnderline styles (SGR 4:x):")
    underline_styles = [
        (0, "No underline"),
        (1, "Straight underline"),
        (2, "Double underline"),
        (3, "Curly underline (for errors)"),
        (4, "Dotted underline"),
        (5, "Dashed underline"),
    ]

    for style_code, style_name in underline_styles:
        term.process_str(f"\x1b[4:{style_code};34m{style_name}\x1b[0m\n")

    print(term.content())


def demonstrate_cursor_styles(term: Terminal):
    """Demonstrate cursor styles."""
    print_header("Cursor Styles (DECSCUSR)")

    term.reset()

    styles = [
        (0, "Default (Blinking Block)"),
        (1, "Blinking Block"),
        (2, "Steady Block"),
        (3, "Blinking Underline"),
        (4, "Steady Underline"),
        (5, "Blinking Bar"),
        (6, "Steady Bar"),
    ]

    print("Available cursor styles:")
    for code, name in styles:
        term.process_str(f"\x1b[{code} q")
        current = term.cursor_style()
        term.process_str(f"  Style {code}: {name} (current: {int(current)})\n")

    print(term.content())


def demonstrate_scrolling_regions(term: Terminal):
    """Demonstrate scrolling regions."""
    print_header("Scrolling Regions (DECSTBM)")

    term.reset()

    # Set scrolling region (rows 5-15)
    term.process_str("\x1b[5;15r")

    print("Scrolling region set to rows 5-15")
    print("Writing 20 lines to demonstrate scrolling...")

    # Move to start of scrolling region
    term.process_str("\x1b[5;1H")

    # Write lines that will scroll within the region
    for i in range(20):
        term.process_str(f"Line {i + 1} in scrolling region\n")

    # Reset scrolling region
    term.process_str("\x1b[r")

    print("\nScrolling region reset to full screen")
    print(term.content())


def demonstrate_alternate_screen(term: Terminal):
    """Demonstrate alternate screen buffer."""
    print_header("Alternate Screen Buffer")

    term.reset()

    # Write to primary screen
    term.process_str("This is the PRIMARY screen.\n")
    term.process_str("It has scrollback history.\n")
    term.process_str("When we switch to alternate screen, this will be hidden.\n")

    print("Primary screen content:")
    print(term.content())
    print(f"Scrollback lines: {term.scrollback_len()}")

    time.sleep(1)

    # Switch to alternate screen
    term.process_str("\x1b[?1049h")

    term.process_str("This is the ALTERNATE screen.\n")
    term.process_str("Used by full-screen apps like vim, less, htop.\n")
    term.process_str("No scrollback in this mode.\n")

    print("\nAlternate screen active:")
    print(term.content())
    print(f"Is alternate screen: {term.is_alt_screen_active()}")

    time.sleep(1)

    # Switch back to primary
    term.process_str("\x1b[?1049l")

    print("\nBack to primary screen:")
    print(term.content())
    print(f"Is alternate screen: {term.is_alt_screen_active()}")


def demonstrate_mouse_tracking(term: Terminal):
    """Demonstrate mouse tracking modes."""
    print_header("Mouse Tracking")

    term.reset()

    modes = [
        ("none", "\x1b[?1000l", "No mouse tracking"),
        ("normal", "\x1b[?1000h", "Normal tracking (clicks)"),
        ("button", "\x1b[?1002h", "Button tracking (drag)"),
        ("any", "\x1b[?1003h", "Any event tracking (movement)"),
    ]

    print("Mouse tracking modes:")
    for mode_name, mode_seq, description in modes:
        term.process_str(mode_seq)
        current_mode = term.mouse_mode()
        print(f"  {mode_name:10s} - {description:30s} (current: {current_mode})")

    # Disable mouse tracking
    term.process_str("\x1b[?1000l")


def demonstrate_clipboard(term: Terminal):
    """Demonstrate OSC 52 clipboard operations."""
    print_header("Clipboard Operations (OSC 52)")

    term.reset()

    import base64

    # Write to clipboard
    text = "Hello from par_term_emu!"
    encoded = base64.b64encode(text.encode()).decode()
    term.process_str(f"\x1b]52;c;{encoded}\x1b\\")

    print(f"Written to clipboard: '{text}'")
    print(f"Clipboard content: '{term.clipboard()}'")

    # Set clipboard programmatically
    term.set_clipboard("Direct clipboard access")
    print(f"After direct set: '{term.clipboard()}'")

    print("\nClipboard features:")
    print("  - Write via OSC 52 escape sequence")
    print("  - Read via clipboard() method")
    print("  - Query via OSC 52 with '?' (security gated)")
    print("  - Works over SSH without X11 forwarding")


def demonstrate_color_queries(term: Terminal):
    """Demonstrate OSC 10/11/12 color queries."""
    print_header("Color Queries (OSC 10/11/12)")

    term.reset()

    # Set custom colors
    term.set_default_fg(255, 128, 64)
    term.set_default_bg(32, 64, 128)
    term.set_cursor_color(0, 255, 0)

    print("Color configuration:")
    fg = term.default_fg()
    bg = term.default_bg()
    cursor = term.cursor_color()

    print(f"  Default Foreground: RGB({fg[0]}, {fg[1]}, {fg[2]})")
    print(f"  Default Background: RGB({bg[0]}, {bg[1]}, {bg[2]})")
    print(f"  Cursor Color:       RGB({cursor[0]}, {cursor[1]}, {cursor[2]})")

    # Query via OSC sequence
    term.process_str("\x1b]10;?\x1b\\")  # Query fg
    responses = term.drain_responses()
    if responses:
        response = bytes(responses).decode('utf-8', errors='ignore')
        print(f"\nQuery response: {response[:50]}...")


def demonstrate_kitty_keyboard(term: Terminal):
    """Demonstrate Kitty keyboard protocol."""
    print_header("Kitty Keyboard Protocol")

    term.reset()

    print("Kitty keyboard protocol features:")
    print("  - Disambiguate escape codes (flag 1)")
    print("  - Report event types (flag 2)")
    print("  - Report alternate key values (flag 4)")
    print("  - Report all keys as escape codes (flag 8)")
    print("  - Report associated text (flag 16)")

    # Set flags
    term.set_keyboard_flags(7, 1)  # Flags 1+2+4
    print(f"\nCurrent flags: {term.keyboard_flags()}")

    # Push/pop example
    print("\nStack operations:")
    print(f"  Initial flags: {term.keyboard_flags()}")

    term.push_keyboard_flags(15)
    print(f"  After push(15): {term.keyboard_flags()}")

    term.pop_keyboard_flags(1)
    print(f"  After pop(1): {term.keyboard_flags()}")


def demonstrate_rectangle_operations(term: Terminal):
    """Demonstrate VT420 rectangle operations."""
    print_header("Rectangle Operations (VT420)")

    term.reset()

    print("VT420 rectangle operations:")

    # Fill rectangle with 'X'
    term.process_str("\x1b[88;5;10;10;20$x")  # Fill with 'X' at (5,10) to (10,20)
    print("\nFilled rectangle with 'X' at rows 5-10, cols 10-20")

    # Draw border
    for col in range(5, 25):
        term.process_str(f"\x1b[3;{col}H─")  # Top border

    for col in range(5, 25):
        term.process_str(f"\x1b[12;{col}H─")  # Bottom border

    print(term.content()[:400] + "...")  # Show partial content


def demonstrate_synchronized_updates(term: Terminal):
    """Demonstrate synchronized updates (DEC 2026)."""
    print_header("Synchronized Updates (DEC 2026)")

    term.reset()

    print("Synchronized updates prevent flicker during complex rendering:")

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")
    print(f"Synchronized updates enabled: {term.synchronized_updates()}")

    # Write complex content (buffered)
    term.process_str("Line 1\n")
    term.process_str("Line 2\n")
    term.process_str("Line 3\n")

    print("Content is buffered (not yet visible)")

    # Disable to flush
    term.process_str("\x1b[?2026l")
    print(f"Synchronized updates disabled: {term.synchronized_updates()}")
    print("Content flushed and now visible")

    print(term.content())


def demonstrate_device_queries(term: Terminal):
    """Demonstrate device query responses."""
    print_header("Device Query Responses")

    term.reset()

    print("Terminal identification via escape sequences:")

    # Primary DA (Device Attributes)
    term.process_str("\x1b[c")
    responses = term.drain_responses()
    if responses:
        response = bytes(responses).decode('utf-8', errors='ignore')
        print(f"  Primary DA: {response}")

    # Secondary DA
    term.process_str("\x1b[>c")
    responses = term.drain_responses()
    if responses:
        response = bytes(responses).decode('utf-8', errors='ignore')
        print(f"  Secondary DA: {response}")

    # Cursor position report (DSR)
    term.process_str("\x1b[6n")
    responses = term.drain_responses()
    if responses:
        response = bytes(responses).decode('utf-8', errors='ignore')
        print(f"  Cursor Position: {response}")


def demonstrate_features_summary():
    """Show summary of all features."""
    print_header("Feature Summary")

    features = [
        ("✓", "VT100/VT220/VT320/VT420 Compatibility"),
        ("✓", "256 colors + True color (24-bit RGB)"),
        ("✓", "Text attributes (bold, italic, underline, etc.)"),
        ("✓", "Six underline styles (SGR 4:x)"),
        ("✓", "Six cursor styles (DECSCUSR)"),
        ("✓", "Scrolling regions (DECSTBM)"),
        ("✓", "Alternate screen buffer"),
        ("✓", "Mouse tracking (4 modes)"),
        ("✓", "Clipboard operations (OSC 52)"),
        ("✓", "Color queries (OSC 10/11/12)"),
        ("✓", "Kitty keyboard protocol"),
        ("✓", "Rectangle operations (VT420)"),
        ("✓", "Synchronized updates (DEC 2026)"),
        ("✓", "Device query responses"),
        ("✓", "Sixel graphics"),
        ("✓", "Shell integration (OSC 133)"),
        ("✓", "Hyperlinks (OSC 8)"),
        ("✓", "Wide characters and emoji"),
    ]

    for check, feature in features:
        print(f"  {check} {feature}")


def main():
    """Run the feature showcase."""
    print("\n" + "=" * 70)
    print("  par_term_emu Feature Showcase")
    print("  Demonstrating terminal emulator capabilities")
    print("=" * 70)

    # Create terminal
    term = Terminal(80, 24)

    try:
        # Run demonstrations
        demonstrate_colors(term)
        time.sleep(0.5)

        demonstrate_text_attributes(term)
        time.sleep(0.5)

        demonstrate_cursor_styles(term)
        time.sleep(0.5)

        demonstrate_scrolling_regions(term)
        time.sleep(0.5)

        demonstrate_alternate_screen(term)
        time.sleep(0.5)

        demonstrate_mouse_tracking(term)
        time.sleep(0.5)

        demonstrate_clipboard(term)
        time.sleep(0.5)

        demonstrate_color_queries(term)
        time.sleep(0.5)

        demonstrate_kitty_keyboard(term)
        time.sleep(0.5)

        demonstrate_rectangle_operations(term)
        time.sleep(0.5)

        demonstrate_synchronized_updates(term)
        time.sleep(0.5)

        demonstrate_device_queries(term)
        time.sleep(0.5)

        demonstrate_features_summary()

        print("\n" + "=" * 70)
        print("  Showcase Complete!")
        print("  For more examples, see the examples/ directory")
        print("=" * 70 + "\n")

    except Exception as e:
        print(f"\nError during showcase: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
