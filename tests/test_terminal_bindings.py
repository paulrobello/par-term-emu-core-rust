"""
Integration tests for Python Terminal bindings

Tests the Python API for the Rust Terminal implementation, including:
- Grid access methods
- Cursor position and state
- Title and mode getters
- Cell attribute queries
- Color operations
- Screenshot generation
"""

import pytest
from par_term_emu_core_rust import Terminal


class TestTerminalBasics:
    """Test basic Terminal creation and properties"""

    def test_create_terminal(self):
        """Test creating a Terminal instance"""
        term = Terminal(80, 24)
        assert term is not None
        assert term.size() == (80, 24)

    def test_create_with_scrollback(self):
        """Test creating a Terminal with custom scrollback"""
        term = Terminal(80, 24, scrollback=5000)
        assert term.size() == (80, 24)
        assert term.scrollback_len() == 0  # Initially empty

    def test_invalid_dimensions(self):
        """Test that zero dimensions raise an error"""
        with pytest.raises(ValueError):
            Terminal(0, 24)

        with pytest.raises(ValueError):
            Terminal(80, 0)

    def test_resize(self):
        """Test resizing the terminal"""
        term = Terminal(80, 24)
        term.resize(100, 30)
        assert term.size() == (100, 30)

    def test_resize_invalid(self):
        """Test that invalid resize raises an error"""
        term = Terminal(80, 24)

        with pytest.raises(ValueError):
            term.resize(0, 24)

        with pytest.raises(ValueError):
            term.resize(80, 0)


class TestCursorOperations:
    """Test cursor position and state"""

    def test_cursor_initial_position(self):
        """Test cursor starts at (0, 0)"""
        term = Terminal(80, 24)
        assert term.cursor_position() == (0, 0)

    def test_cursor_movement(self):
        """Test cursor movement via escape sequences"""
        term = Terminal(80, 24)

        # Move to (10, 15) using CSI H (1-indexed)
        term.process_str("\x1b[16;11H")
        assert term.cursor_position() == (10, 15)

        # Move using other sequences
        term.process_str("\x1b[A")  # Up
        assert term.cursor_position() == (10, 14)

        term.process_str("\x1b[5C")  # Right 5
        assert term.cursor_position() == (15, 14)

    def test_cursor_visibility(self):
        """Test cursor visibility state"""
        term = Terminal(80, 24)

        # Show cursor (default)
        term.process_str("\x1b[?25h")
        # Can't directly query visibility, but operation should not crash

        # Hide cursor
        term.process_str("\x1b[?25l")

    def test_cursor_style(self):
        """Test setting cursor style"""
        term = Terminal(80, 24)

        # Set various cursor styles
        term.process_str("\x1b[1 q")  # Blinking block
        term.process_str("\x1b[2 q")  # Steady block
        term.process_str("\x1b[3 q")  # Blinking underline
        term.process_str("\x1b[4 q")  # Steady underline
        term.process_str("\x1b[5 q")  # Blinking bar
        term.process_str("\x1b[6 q")  # Steady bar


class TestContentOperations:
    """Test content reading and querying"""

    def test_write_simple_text(self):
        """Test writing and reading simple text"""
        term = Terminal(80, 24)
        term.process_str("Hello, World!")

        content = term.content()
        assert "Hello, World!" in content

    def test_write_with_newlines(self):
        """Test writing text with newlines"""
        term = Terminal(80, 24)
        term.process_str("Line 1\nLine 2\nLine 3")

        content = term.content()
        assert "Line 1" in content
        assert "Line 2" in content
        assert "Line 3" in content

    def test_get_line(self):
        """Test getting specific lines"""
        term = Terminal(80, 24)
        term.process_str("\x1b[1HFirst Line")
        term.process_str("\x1b[2HSecond Line")
        term.process_str("\x1b[3HThird Line")

        line0 = term.get_line(0)
        line1 = term.get_line(1)
        line2 = term.get_line(2)

        assert "First Line" in line0
        assert "Second Line" in line1
        assert "Third Line" in line2

    def test_get_line_invalid(self):
        """Test getting line with invalid row returns None"""
        term = Terminal(80, 24)
        assert term.get_line(100) is None
        # Note: negative indices raise OverflowError, not return None

    def test_get_char(self):
        """Test getting individual characters"""
        term = Terminal(80, 24)
        term.process_str("ABC")

        assert term.get_char(0, 0) == "A"
        assert term.get_char(1, 0) == "B"
        assert term.get_char(2, 0) == "C"
        assert term.get_char(3, 0) == " "  # Empty cells are spaces

    def test_get_char_invalid(self):
        """Test getting char with invalid position returns None"""
        term = Terminal(80, 24)
        assert term.get_char(100, 0) is None
        assert term.get_char(0, 100) is None


class TestColorOperations:
    """Test color operations and queries"""

    def test_get_fg_color(self):
        """Test getting foreground colors"""
        term = Terminal(80, 24)

        # Write with red foreground
        term.process_str("\x1b[31mRed")

        # Get color at position
        color = term.get_fg_color(0, 0)
        assert color is not None
        # Color is returned as (r, g, b) tuple or named color index

    def test_get_bg_color(self):
        """Test getting background colors"""
        term = Terminal(80, 24)

        # Write with green background
        term.process_str("\x1b[42mGreen BG")

        color = term.get_bg_color(0, 0)
        assert color is not None

    def test_rgb_colors(self):
        """Test RGB color support"""
        term = Terminal(80, 24)

        # Set RGB foreground color
        term.process_str("\x1b[38;2;255;128;64mRGB Text")

        # Get color (should be RGB)
        color = term.get_fg_color(0, 0)
        assert color is not None

    def test_256_colors(self):
        """Test 256-color palette support"""
        term = Terminal(80, 24)

        # Set 256-color foreground
        term.process_str("\x1b[38;5;123m256 Color")

        color = term.get_fg_color(0, 0)
        assert color is not None

    def test_color_reset(self):
        """Test color reset to defaults"""
        term = Terminal(80, 24)

        # Set colors
        term.process_str("\x1b[31;42mColored")

        # Reset
        term.process_str("\x1b[0mReset")

        # Colors should be back to defaults


class TestCellAttributes:
    """Test cell attribute queries"""

    def test_get_attributes_bold(self):
        """Test getting bold attribute"""
        term = Terminal(80, 24)

        # Write with bold
        term.process_str("\x1b[1mBold")

        attrs = term.get_attributes(0, 0)
        assert attrs is not None
        # Attributes is a custom type with specific fields
        assert hasattr(attrs, "bold") or hasattr(attrs, "is_bold")

    def test_get_attributes_italic(self):
        """Test getting italic attribute"""
        term = Terminal(80, 24)

        term.process_str("\x1b[3mItalic")

        attrs = term.get_attributes(0, 0)
        assert attrs is not None

    def test_get_attributes_underline(self):
        """Test getting underline attribute"""
        term = Terminal(80, 24)

        term.process_str("\x1b[4mUnderline")

        attrs = term.get_attributes(0, 0)
        assert attrs is not None

    def test_get_attributes_multiple(self):
        """Test multiple attributes combined"""
        term = Terminal(80, 24)

        term.process_str("\x1b[1;3;4mBold Italic Underline")

        attrs = term.get_attributes(0, 0)
        assert attrs is not None


class TestTitleOperations:
    """Test window title operations"""

    def test_title_initial(self):
        """Test initial title is empty"""
        term = Terminal(80, 24)
        assert term.title() == ""

    def test_set_title_osc0(self):
        """Test setting title with OSC 0"""
        term = Terminal(80, 24)
        term.process_str("\x1b]0;Test Title\x07")
        assert term.title() == "Test Title"

    def test_set_title_osc2(self):
        """Test setting title with OSC 2"""
        term = Terminal(80, 24)
        term.process_str("\x1b]2;Another Title\x07")
        assert term.title() == "Another Title"

    def test_title_with_unicode(self):
        """Test title with Unicode characters"""
        term = Terminal(80, 24)
        term.process_str("\x1b]0;测试标题\x07")
        assert term.title() == "测试标题"

    def test_title_stack(self):
        """Test title push/pop stack"""
        term = Terminal(80, 24)

        term.process_str("\x1b]0;Original\x07")
        assert term.title() == "Original"

        # Push title
        term.process_str("\x1b]21\x07")

        # Change title
        term.process_str("\x1b]0;New\x07")
        assert term.title() == "New"

        # Pop title
        term.process_str("\x1b]22\x07")
        assert term.title() == "Original"


class TestScrollback:
    """Test scrollback operations"""

    def test_scrollback_initial_empty(self):
        """Test scrollback is initially empty"""
        term = Terminal(80, 24)
        assert term.scrollback_len() == 0
        assert term.scrollback() == []

    def test_scrollback_fills_on_scroll(self):
        """Test scrollback fills when terminal scrolls"""
        term = Terminal(80, 5)  # Small terminal

        # Write more lines than terminal height
        for i in range(10):
            term.process_str(f"Line {i}\n")

        # Should have scrollback
        scrollback_len = term.scrollback_len()
        assert scrollback_len > 0

    def test_scrollback_content(self):
        """Test scrollback content retrieval"""
        term = Terminal(80, 5, scrollback=100)

        # Write lines to fill scrollback
        for i in range(10):
            term.process_str(f"Line {i}\n")

        scrollback = term.scrollback()
        assert len(scrollback) > 0
        # Earlier lines should be in scrollback


class TestModesAndModes:
    """Test terminal modes and state"""

    def test_alt_screen_mode(self):
        """Test alternate screen buffer"""
        term = Terminal(80, 24)

        # Write to main screen
        term.process_str("Main Screen")

        # Switch to alt screen
        term.process_str("\x1b[?1049h")

        # Alt screen should be empty
        content = term.content()
        assert "Main Screen" not in content

        # Switch back
        term.process_str("\x1b[?1049l")

        # Main screen content should be back
        content = term.content()
        assert "Main Screen" in content

    def test_application_cursor_mode(self):
        """Test application cursor mode"""
        term = Terminal(80, 24)

        # Enable application cursor
        term.process_str("\x1b[?1h")

        # Disable
        term.process_str("\x1b[?1l")

    def test_mouse_tracking_mode(self):
        """Test mouse tracking modes"""
        term = Terminal(80, 24)

        # Enable various mouse modes
        term.process_str("\x1b[?1000h")  # Normal tracking
        term.process_str("\x1b[?1002h")  # Button event
        term.process_str("\x1b[?1003h")  # Any event

        # Disable
        term.process_str("\x1b[?1000l")

    def test_bracketed_paste_mode(self):
        """Test bracketed paste mode"""
        term = Terminal(80, 24)

        # Enable bracketed paste
        term.process_str("\x1b[?2004h")

        # Disable
        term.process_str("\x1b[?2004l")


class TestHyperlinks:
    """Test hyperlink operations"""

    def test_hyperlink_basic(self):
        """Test basic hyperlink support"""
        term = Terminal(80, 24)

        # Set hyperlink
        term.process_str("\x1b]8;;https://example.com\x07Click\x1b]8;;\x07")

        # Get hyperlink at position
        url = term.get_hyperlink(0, 0)
        assert url == "https://example.com"

    def test_hyperlink_end(self):
        """Test ending a hyperlink"""
        term = Terminal(80, 24)

        term.process_str("\x1b]8;;https://example.com\x07Link\x1b]8;;\x07 No Link")

        # Link at start
        assert term.get_hyperlink(0, 0) == "https://example.com"

        # No link after end
        assert term.get_hyperlink(5, 0) is None

    def test_hyperlink_none_when_empty(self):
        """Test get_hyperlink returns None when no link"""
        term = Terminal(80, 24)
        term.process_str("No link here")

        assert term.get_hyperlink(0, 0) is None


class TestReset:
    """Test terminal reset operations"""

    def test_reset_clears_state(self):
        """Test that reset clears terminal state"""
        term = Terminal(80, 24)

        # Set up some state
        term.process_str("\x1b[10;20H")  # Move cursor
        term.process_str("\x1b[31m")  # Red fg
        term.process_str("\x1b[1m")  # Bold
        term.process_str("Some text")

        # Reset
        term.process_str("\x1bc")

        # Cursor should be at origin
        assert term.cursor_position() == (0, 0)

    def test_reset_clears_content(self):
        """Test that clear (not reset) clears content"""
        term = Terminal(80, 24)
        term.process_str("Some content")

        # Clear screen
        term.process_str("\x1b[2J")

        content = term.content()
        # Should be mostly empty (spaces)
        assert "Some content" not in content


class TestSpecialFeatures:
    """Test special features"""

    def test_focus_events(self):
        """Test focus event sequences"""
        term = Terminal(80, 24)

        focus_in = term.get_focus_in_event()
        focus_out = term.get_focus_out_event()

        # Methods exist and return bytes
        assert isinstance(focus_in, bytes)
        assert isinstance(focus_out, bytes)

    def test_repr_and_str(self):
        """Test __repr__ and __str__ methods"""
        term = Terminal(80, 24)

        repr_str = repr(term)
        assert "Terminal" in repr_str
        assert "80" in repr_str
        assert "24" in repr_str

        str_str = str(term)
        assert isinstance(str_str, str)

    def test_process_bytes(self):
        """Test processing bytes instead of strings"""
        term = Terminal(80, 24)

        # Process bytes directly
        term.process(b"Hello")
        term.process(b"\x1b[31m")  # Red
        term.process(b"World")

        content = term.content()
        assert "Hello" in content
        assert "World" in content


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
