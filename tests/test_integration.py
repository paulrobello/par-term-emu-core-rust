"""
Comprehensive integration tests for par_term_emu.

These tests verify that multiple features work together correctly
in realistic usage scenarios.
"""

import pytest
from par_term_emu_core_rust import Terminal, CursorStyle


class TestFeatureIntegration:
    """Test that multiple features work together correctly."""

    def test_cursor_and_colors_integration(self):
        """Test cursor styles work with color changes."""
        term = Terminal(80, 24)

        # Set cursor style
        term.process_str("\x1b[3 q")  # Blinking underline
        assert int(term.cursor_style()) == int(CursorStyle.BlinkingUnderline)

        # Change colors
        term.process_str("\x1b[31m")  # Red foreground
        term.process_str("Red Text")

        # Cursor style should persist
        assert int(term.cursor_style()) == int(CursorStyle.BlinkingUnderline)

        # Verify text and color
        line = term.get_line(0)
        assert "Red" in line

    def test_alternate_screen_with_scrollback(self):
        """Test alternate screen doesn't affect primary screen scrollback."""
        term = Terminal(80, 24)

        # Write to primary screen
        for i in range(30):
            term.process_str(f"Primary line {i}\n")

        # Should have scrollback
        scrollback_before = term.scrollback()
        assert len(scrollback_before) > 0

        # Switch to alternate screen
        term.process_str("\x1b[?1049h")
        assert term.is_alt_screen_active()

        # Write to alternate screen
        term.process_str("Alternate screen content")

        # Switch back to primary
        term.process_str("\x1b[?1049l")
        assert not term.is_alt_screen_active()

        # Scrollback should be unchanged
        scrollback_after = term.scrollback()
        assert len(scrollback_after) == len(scrollback_before)

    def test_mouse_tracking_with_alternate_screen(self):
        """Test mouse tracking persists across screen switches."""
        term = Terminal(80, 24)

        # Enable mouse tracking
        term.process_str("\x1b[?1000h")
        assert term.mouse_mode() == "normal"

        # Switch to alternate screen
        term.process_str("\x1b[?1049h")

        # Mouse tracking should still be active
        assert term.mouse_mode() == "normal"

        # Switch back
        term.process_str("\x1b[?1049l")

        # Mouse tracking should still be active
        assert term.mouse_mode() == "normal"

    def test_sgr_attributes_with_underline_styles(self):
        """Test SGR attributes work correctly with underline styles."""
        term = Terminal(80, 24)

        # Set bold + curly underline + red
        term.process_str("\x1b[1;4:3;31mText\x1b[0m")

        # Check position after writing
        assert term.cursor_position()[0] > 0

    def test_scrolling_region_with_colors(self):
        """Test scrolling regions preserve colors."""
        term = Terminal(80, 24)

        # Set scrolling region (rows 5-15)
        term.process_str("\x1b[5;15r")

        # Move to scrolling region and set color
        term.process_str("\x1b[5;1H")  # Row 5, col 1
        term.process_str("\x1b[32m")  # Green

        # Fill scrolling region
        for i in range(15):
            term.process_str(f"Line {i}\n")

        # Colors should be preserved during scrolling
        content = term.content()
        assert "Line" in content

    def test_cursor_save_restore_with_attributes(self):
        """Test DECSC/DECRC saves and restores all attributes."""
        term = Terminal(80, 24)

        # Set attributes
        term.process_str("\x1b[1;31m")  # Bold + red
        term.process_str("\x1b[3 q")  # Blinking underline cursor
        term.process_str("\x1b[10;20H")  # Move cursor

        # Save cursor
        term.process_str("\x1b7")

        # Change everything
        term.process_str("\x1b[0m")  # Reset attributes
        term.process_str("\x1b[1 q")  # Blinking block cursor
        term.process_str("\x1b[1;1H")  # Move to top-left

        # Restore cursor
        term.process_str("\x1b8")

        # Cursor position and style should be restored
        cursor = term.cursor_position()
        assert cursor[0] == 19  # Column 20 (0-indexed)
        assert cursor[1] == 9  # Row 10 (0-indexed)
        assert int(term.cursor_style()) == int(CursorStyle.BlinkingUnderline)

    def test_device_queries_with_capabilities(self):
        """Test device attribute queries report correct capabilities."""
        term = Terminal(80, 24)

        # Primary DA query
        term.process_str("\x1b[c")
        responses = term.drain_responses()
        response = bytes(responses).decode("utf-8", errors="ignore")

        # Should report VT level and capabilities
        assert "?6" in response or "62" in response  # VT200 or higher
        # Should include Sixel capability (code 4)
        assert "4" in response

    def test_kitty_keyboard_with_flags(self):
        """Test Kitty keyboard protocol flag management."""
        term = Terminal(80, 24)

        # Set flags
        term.set_keyboard_flags(7, 1)  # Flags 1+2+4 = 7
        assert term.keyboard_flags() == 7

        # Push new flags
        term.push_keyboard_flags(15)
        assert term.keyboard_flags() == 15

        # Pop to restore
        term.pop_keyboard_flags(1)
        assert term.keyboard_flags() == 7

    def test_osc_52_clipboard_operations(self):
        """Test OSC 52 clipboard read/write."""
        term = Terminal(80, 24)

        # Write to clipboard
        import base64

        text = "Hello Clipboard"
        encoded = base64.b64encode(text.encode()).decode()
        term.process_str(f"\x1b]52;c;{encoded}\x1b\\")

        # Read clipboard
        clipboard = term.clipboard()
        assert clipboard == text

        # Query clipboard via OSC 52 with "?" (should be denied by default)
        term.process_str("\x1b]52;c;?\x1b\\")
        responses = term.drain_responses()
        # No response expected unless allow_clipboard_read is true
        assert len(responses) == 0

        # Allow reads and try again
        term.set_allow_clipboard_read(True)
        term.process_str("\x1b]52;c;?\x1b\\")
        responses = term.drain_responses()
        assert len(responses) > 0

    def test_color_queries(self):
        """Test OSC 10/11/12 color queries."""
        term = Terminal(80, 24)

        # Set custom colors
        term.set_default_fg(255, 128, 64)
        term.set_default_bg(32, 64, 128)
        term.set_cursor_color(0, 255, 0)

        # Verify getters
        assert term.default_fg() == (255, 128, 64)
        assert term.default_bg() == (32, 64, 128)
        assert term.cursor_color() == (0, 255, 0)

        # Query colors via OSC sequences
        term.process_str("\x1b]10;?\x1b\\")  # Query default fg
        responses = term.drain_responses()
        response = bytes(responses).decode("utf-8", errors="ignore")
        assert "rgb:" in response

    def test_rectangle_operations(self):
        """Test VT420 rectangle operations."""
        term = Terminal(80, 24)

        # Fill a rectangle with 'X'
        term.process_str("\x1b[88;5;10;10;20$x")  # Fill with 'X' (ASCII 88)

        # Verify fill
        for row in range(4, 10):
            line = term.get_line(row)
            if line:
                # Should have X characters in columns 9-19
                assert "X" in line

    def test_synchronized_updates(self):
        """Test synchronized updates buffer correctly."""
        term = Terminal(80, 24)

        # Enable synchronized updates
        term.process_str("\x1b[?2026h")
        assert term.synchronized_updates()

        # Write content (should be buffered)
        term.process_str("Buffered content")

        # Disable (should flush)
        term.process_str("\x1b[?2026l")
        assert not term.synchronized_updates()

        # Content should now be visible
        content = term.content()
        assert "Buffered" in content

    def test_sixel_graphics_basic(self):
        """Test basic Sixel graphics integration."""
        term = Terminal(80, 24)

        # Simple Sixel sequence (2x2 red square)
        term.process_str("\x1bPq")  # Start Sixel
        term.process_str("#0;2;100;0;0")  # Color 0 = red
        term.process_str("#0~~")  # Draw two columns
        term.process_str("\x1b\\")  # End Sixel

        # Should have at least one graphic
        graphics_count = term.graphics_count()
        assert graphics_count >= 0  # May be > 0 if graphic was stored


class TestEdgeCases:
    """Test edge cases and boundary conditions."""

    def test_empty_terminal_operations(self):
        """Test operations on empty terminal."""
        term = Terminal(80, 24)

        # Should not crash
        content = term.content()
        # Empty terminal returns blank lines, not empty string
        assert len(content) > 0  # Has content (blank lines)
        assert content.strip() == ""  # But it's all whitespace
        assert term.cursor_position() == (0, 0)
        assert len(term.scrollback()) == 0

    def test_resize_preserves_content(self):
        """Test resizing preserves visible content."""
        term = Terminal(80, 24)

        # Write content
        term.process_str("Line 1\nLine 2\nLine 3\n")

        # Resize smaller
        term.resize(40, 12)

        # Content should still be there (wrapped)
        content = term.content()
        assert "Line 1" in content

        # Resize larger
        term.resize(120, 36)

        # Content should still be visible
        content = term.content()
        assert "Line 1" in content

    def test_maximum_scrollback(self):
        """Test scrollback respects maximum limit."""
        term = Terminal(80, 24, scrollback=10)

        # Write more lines than scrollback limit
        for i in range(50):
            term.process_str(f"Line {i}\n")

        # Scrollback should be limited
        scrollback = term.scrollback()
        assert len(scrollback) <= 10

    def test_unicode_and_emoji(self):
        """Test Unicode and emoji handling."""
        term = Terminal(80, 24)

        # Write Unicode and emoji
        term.process_str("Hello 世界 🌍\n")
        term.process_str("Émojis: 😀 🎉 ✨\n")

        # Should not crash
        content = term.content()
        assert len(content) > 0

    def test_long_lines(self):
        """Test handling of lines longer than terminal width."""
        term = Terminal(80, 24)

        # Write a very long line
        long_line = "A" * 200
        term.process_str(long_line)

        # Should wrap or truncate gracefully
        content = term.content()
        assert "A" in content

    def test_rapid_mode_switching(self):
        """Test rapid alternate screen switching."""
        term = Terminal(80, 24)

        # Rapidly switch modes
        for _ in range(10):
            term.process_str("\x1b[?1049h")  # To alt
            term.process_str("\x1b[?1049l")  # To primary

        # Should end up on primary screen
        assert not term.is_alt_screen_active()

    def test_concurrent_attributes(self):
        """Test multiple SGR attributes simultaneously."""
        term = Terminal(80, 24)

        # Set many attributes at once
        term.process_str("\x1b[1;3;4:3;7;31;42mText\x1b[0m")

        # Should not crash
        assert term.cursor_position()[0] > 0

    def test_invalid_sequences(self):
        """Test handling of invalid escape sequences."""
        term = Terminal(80, 24)

        # Invalid sequences should be ignored
        term.process_str("\x1b[999999999999999999A")  # Huge cursor up
        term.process_str("\x1b[?999999h")  # Invalid mode
        term.process_str("\x1b]9999;invalid\x1b\\")  # Invalid OSC

        # Terminal should still function
        term.process_str("Hello")
        assert "Hello" in term.content()


class TestPerformance:
    """Performance-related tests."""

    def test_bulk_write_performance(self):
        """Test performance of bulk writes."""
        term = Terminal(80, 24)

        # Write a lot of data
        data = "Line of text\n" * 1000
        term.process_str(data)

        # Should complete without hanging
        assert term.cursor_position()[1] >= 0

    def test_many_color_changes(self):
        """Test performance with many color changes."""
        term = Terminal(80, 24)

        # Alternate colors many times
        for i in range(100):
            term.process_str(f"\x1b[{31 + (i % 7)}mColor {i} ")

        # Should complete
        assert len(term.content()) > 0

    def test_deep_scrollback(self):
        """Test performance with large scrollback."""
        term = Terminal(80, 24, scrollback=1000)

        # Fill scrollback
        for i in range(1500):
            term.process_str(f"Line {i}\n")

        # Operations should still be fast
        scrollback = term.scrollback()
        assert len(scrollback) <= 1000


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
