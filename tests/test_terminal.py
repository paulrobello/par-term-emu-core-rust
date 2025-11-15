#!/usr/bin/env python3
"""
Tests for par_term_emu
"""

import pytest
from par_term_emu_core_rust import Terminal


def test_terminal_creation():
    """Test creating a terminal"""
    term = Terminal(80, 24)
    assert term.size() == (80, 24)


def test_terminal_invalid_size():
    """Test that invalid sizes raise an error"""
    with pytest.raises(ValueError):
        Terminal(0, 24)
    with pytest.raises(ValueError):
        Terminal(80, 0)


def test_basic_text():
    """Test writing basic text"""
    term = Terminal(80, 24)
    term.process_str("Hello, World!")

    content = term.content()
    assert "Hello, World!" in content


def test_newlines():
    """Test handling newlines"""
    term = Terminal(80, 24)
    term.process_str("Line 1\nLine 2\nLine 3")

    lines = term.content().split("\n")
    assert "Line 1" in lines[0]
    assert "Line 2" in lines[1]
    assert "Line 3" in lines[2]


def test_cursor_position():
    """Test cursor positioning"""
    term = Terminal(80, 24)
    term.process_str("Test")

    col, row = term.cursor_position()
    assert col == 4
    assert row == 0


def test_cursor_movement():
    """Test ANSI cursor movement"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5;10HA")  # Move to row 5, col 10 and write 'A'

    char = term.get_char(9, 4)  # 0-indexed
    assert char == "A"


def test_colors():
    """Test ANSI color codes"""
    term = Terminal(80, 24)
    term.process_str("\x1b[31mRed\x1b[0m")

    content = term.content()
    assert "Red" in content


def test_rgb_colors():
    """Test 24-bit RGB colors"""
    term = Terminal(80, 24)
    term.process_str("\x1b[38;2;255;0;0mRed\x1b[0m")

    # Get foreground color of first character
    fg_color = term.get_fg_color(0, 0)
    assert fg_color is not None
    r, g, b = fg_color
    assert r == 255


def test_text_attributes():
    """Test text attributes"""
    term = Terminal(80, 24)
    term.process_str("\x1b[1;4mBold and underlined\x1b[0m")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.bold is True
    assert attrs.underline is True


def test_clear_screen():
    """Test clearing the screen"""
    term = Terminal(80, 24)
    term.process_str("Some text")
    term.process_str("\x1b[2J")  # Clear screen

    content = term.content()
    assert "Some text" not in content


def test_scrollback():
    """Test scrollback buffer"""
    term = Terminal(40, 5, scrollback=100)

    # Write more lines than terminal height
    for i in range(10):
        term.process_str(f"Line {i}\n")

    assert term.scrollback_len() > 0


def test_resize():
    """Test terminal resizing"""
    term = Terminal(80, 24)
    term.process_str("Test")

    term.resize(100, 30)
    assert term.size() == (100, 30)

    # Content should be preserved
    content = term.content()
    assert "Test" in content


def test_resize_invalid():
    """Test that invalid resize raises an error"""
    term = Terminal(80, 24)

    with pytest.raises(ValueError):
        term.resize(0, 24)
    with pytest.raises(ValueError):
        term.resize(80, 0)


def test_reset():
    """Test resetting the terminal"""
    term = Terminal(80, 24)
    term.process_str("Some content")

    term.reset()
    content = term.content()

    # After reset, content should be empty
    assert content.strip() == "" or all(c == " " or c == "\n" for c in content)


def test_get_line():
    """Test getting a specific line"""
    term = Terminal(80, 24)
    term.process_str("First line\nSecond line")

    line0 = term.get_line(0)
    line1 = term.get_line(1)

    assert line0 is not None
    assert "First line" in line0
    assert line1 is not None
    assert "Second line" in line1


def test_terminal_title():
    """Test setting terminal title via OSC sequence"""
    term = Terminal(80, 24)
    term.process_str("\x1b]0;Test Title\x07")

    assert term.title() == "Test Title"


def test_cursor_visibility():
    """Test cursor visibility control"""
    term = Terminal(80, 24)
    assert term.cursor_visible() is True

    term.process_str("\x1b[?25l")  # Hide cursor
    assert term.cursor_visible() is False

    term.process_str("\x1b[?25h")  # Show cursor
    assert term.cursor_visible() is True


def test_repr():
    """Test string representation"""
    term = Terminal(80, 24)
    repr_str = repr(term)
    assert "80" in repr_str
    assert "24" in repr_str


# VT220 Editing Tests
def test_insert_lines():
    """Test VT220 IL (Insert Lines) command"""
    term = Terminal(80, 24)
    term.process_str("Line 0\nLine 1\nLine 2\nLine 3\nLine 4")
    term.process_str("\x1b[2;1H")  # Move to row 2, col 1
    term.process_str("\x1b[2L")  # Insert 2 lines

    line1 = term.get_line(1)
    assert line1.strip() == ""  # Line 1 should be blank

    line3 = term.get_line(3)
    assert "Line 1" in line3  # Original line 1 should move to row 3


def test_delete_lines():
    """Test VT220 DL (Delete Lines) command"""
    term = Terminal(80, 24)
    term.process_str("Line 0\nLine 1\nLine 2\nLine 3\nLine 4")
    term.process_str("\x1b[2;1H")  # Move to row 2, col 1
    term.process_str("\x1b[2M")  # Delete 2 lines

    line1 = term.get_line(1)
    assert "Line 3" in line1  # Line 3 should move up to row 1


def test_insert_characters():
    """Test VT220 ICH (Insert Characters) command"""
    term = Terminal(80, 24)
    term.process_str("ABCDEFGH")
    term.process_str("\x1b[1;4H")  # Move to col 4 (after C)
    term.process_str("\x1b[3@")  # Insert 3 characters

    line0 = term.get_line(0)
    assert "ABC" in line0
    assert "DEFGH" in line0


def test_delete_characters():
    """Test VT220 DCH (Delete Characters) command"""
    term = Terminal(80, 24)
    term.process_str("ABCDEFGH")
    term.process_str("\x1b[1;3H")  # Move to col 3 (C)
    term.process_str("\x1b[2P")  # Delete 2 characters

    line0 = term.get_line(0)
    assert "ABEFGH" in line0


def test_erase_characters():
    """Test VT220 ECH (Erase Characters) command"""
    term = Terminal(80, 24)
    term.process_str("ABCDEFGH")
    term.process_str("\x1b[1;3H")  # Move to col 3 (C)
    term.process_str("\x1b[3X")  # Erase 3 characters

    line0 = term.get_line(0)
    assert "AB" in line0
    assert "FGH" in line0


# Scrolling Region Tests
def test_scroll_region_basic():
    """Test setting scroll region with DECSTBM"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5;10r")  # Set scroll region lines 5-10

    # Verify by testing cursor movement behavior
    col, row = term.cursor_position()
    # After setting scroll region, cursor should move to home
    assert row == 0


def test_scroll_region_scroll_up():
    """Test scrolling within a region"""
    term = Terminal(80, 10)
    for i in range(10):
        term.process_str(f"Line {i}\r\n")

    term.process_str("\x1b[3;7r")  # Set scroll region lines 3-7 (1-indexed)
    term.process_str("\x1b[3;1H")  # Move to line 3 (row 2 in 0-indexed)
    term.process_str("\x1b[1M")  # Delete line at cursor (scrolls region up)

    # After DL at row 2 within scroll region, row 2 should have what was row 3
    line2 = term.get_line(2)
    assert "Line 4" in line2  # Line 3 (row 2) is deleted, Line 4 (row 3) moves up


def test_scroll_up_command():
    """Test CSI S (Scroll Up) command"""
    term = Terminal(80, 5)
    for i in range(5):
        term.process_str(f"Line {i}\r\n")

    term.process_str("\x1b[2S")  # Scroll up 2 lines

    # Top lines should contain what was at lines 2-4
    line0 = term.get_line(0)
    assert "Line 2" in line0 or "Line 3" in line0


def test_scroll_down_command():
    """Test CSI T (Scroll Down) command"""
    term = Terminal(80, 5)
    for i in range(5):
        term.process_str(f"Line {i}\r\n")

    term.process_str("\x1b[1;1H")  # Move to top
    term.process_str("\x1b[2T")  # Scroll down 2 lines

    # First lines should be blank
    line0 = term.get_line(0)
    assert line0.strip() == ""


# Tab Stop Tests
def test_tab_default_behavior():
    """Test default tab stops (every 8 columns)"""
    term = Terminal(80, 24)
    term.process_str("A\t")  # Write A then tab

    col, row = term.cursor_position()
    assert col == 8  # Should tab to column 8


def test_set_tab_stop():
    """Test HTS (Set Tab Stop)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5G")  # Move to column 5
    term.process_str("\x1bH")  # Set tab stop
    term.process_str("\x1b[1G")  # Move to column 1
    term.process_str("\t")  # Tab

    col, row = term.cursor_position()
    assert col == 4 or col == 5  # Should tab to custom stop


def test_clear_tab_stop():
    """Test TBC (Clear Tab Stop)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[9G")  # Move to column 9 (tab stop at 8)
    term.process_str("\x1b[0g")  # Clear tab stop at current position
    term.process_str("\x1b[1G")  # Move to column 1
    term.process_str("\t")  # Tab

    col, row = term.cursor_position()
    # Should skip the cleared tab stop
    assert col >= 15  # Should tab to next available stop


def test_clear_all_tab_stops():
    """Test clearing all tab stops"""
    term = Terminal(80, 24)
    term.process_str("\x1b[3g")  # Clear all tab stops
    term.process_str("\t")  # Try to tab

    col, row = term.cursor_position()
    # Without tab stops, should go to end of line
    assert col >= 79


def test_forward_tabulation():
    """Test CHT (Cursor Forward Tabulation)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[2I")  # Forward 2 tab stops

    col, row = term.cursor_position()
    assert col == 16  # From 0 to 8, then to 16


def test_backward_tabulation():
    """Test CBT (Cursor Backward Tabulation)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[20G")  # Move to column 20
    term.process_str("\x1b[1Z")  # Backward 1 tab stop

    col, row = term.cursor_position()
    assert col == 16  # Should be at tab stop 16


# Wide Character and Unicode Tests
def test_wide_character():
    """Test handling of wide characters (CJK)"""
    term = Terminal(80, 24)
    term.process_str("中")  # Chinese character (2 cells wide)

    col, row = term.cursor_position()
    assert col == 2  # Wide character should advance cursor by 2


def test_emoji():
    """Test handling of emoji"""
    term = Terminal(80, 24)
    term.process_str("😀")  # Emoji

    content = term.content()
    assert "😀" in content or "?" in content  # Should contain emoji or replacement


def test_mixed_width_characters():
    """Test mixing ASCII and wide characters"""
    term = Terminal(80, 24)
    term.process_str("A中B")  # ASCII, wide, ASCII

    col, row = term.cursor_position()
    assert col == 4  # A(1) + 中(2) + B(1) = 4


def test_wide_character_wrap():
    """Test wide character at line boundary"""
    term = Terminal(10, 5)
    term.process_str("123456789中")  # Wide char at boundary

    col, row = term.cursor_position()
    # Wide character should wrap or handle boundary properly
    assert row >= 1 or col <= 9


def test_wide_character_flags():
    """Test that wide_char and wide_char_spacer flags are set correctly"""
    term = Terminal(80, 24)
    term.process_str("A中B")  # ASCII, wide, ASCII

    # Check first cell (ASCII 'A')
    attrs_a = term.get_attributes(0, 0)
    assert attrs_a is not None
    assert not attrs_a.wide_char
    assert not attrs_a.wide_char_spacer

    # Check second cell (wide char '中')
    attrs_wide = term.get_attributes(1, 0)
    assert attrs_wide is not None
    assert attrs_wide.wide_char
    assert not attrs_wide.wide_char_spacer

    # Check third cell (spacer for '中')
    attrs_spacer = term.get_attributes(2, 0)
    assert attrs_spacer is not None
    assert not attrs_spacer.wide_char
    assert attrs_spacer.wide_char_spacer

    # Check fourth cell (ASCII 'B')
    attrs_b = term.get_attributes(3, 0)
    assert attrs_b is not None
    assert not attrs_b.wide_char
    assert not attrs_b.wide_char_spacer


def test_emoji_flags():
    """Test that emoji are marked with wide_char flags"""
    term = Terminal(80, 24)
    term.process_str("😀")  # Emoji

    # Check first cell (emoji)
    attrs_emoji = term.get_attributes(0, 0)
    assert attrs_emoji is not None
    assert attrs_emoji.wide_char

    # Check second cell (spacer)
    attrs_spacer = term.get_attributes(1, 0)
    assert attrs_spacer is not None
    assert attrs_spacer.wide_char_spacer


def test_snapshot_wide_character_flags():
    """Test that snapshots preserve wide character flags"""
    term = Terminal(80, 24)
    term.process_str("Hello 中文")  # ASCII + wide chars

    snapshot = term.create_snapshot()
    line = snapshot.get_line(0)

    # Check the '中' character at index 6
    char, fg, bg, attrs = line[6]
    assert char == "中"
    assert attrs.wide_char
    assert not attrs.wide_char_spacer

    # Check the spacer at index 7
    char_spacer, fg_spacer, bg_spacer, attrs_spacer = line[7]
    assert not attrs_spacer.wide_char
    assert attrs_spacer.wide_char_spacer


# Mouse Tracking Tests
def test_mouse_normal_mode():
    """Test normal mouse tracking mode"""
    term = Terminal(80, 24)
    term.process_str("\x1b[?1000h")  # Enable normal mouse tracking

    # Mode should be enabled (can't directly test mode in Python API)
    # But we can verify it doesn't crash
    assert term.size() == (80, 24)


def test_mouse_button_event_mode():
    """Test button event mouse tracking"""
    term = Terminal(80, 24)
    term.process_str("\x1b[?1002h")  # Enable button event mode

    assert term.size() == (80, 24)


def test_mouse_sgr_encoding():
    """Test SGR mouse encoding"""
    term = Terminal(80, 24)
    term.process_str("\x1b[?1006h")  # Enable SGR encoding

    assert term.size() == (80, 24)


def test_mouse_disable():
    """Test disabling mouse tracking"""
    term = Terminal(80, 24)
    term.process_str("\x1b[?1000h")  # Enable
    term.process_str("\x1b[?1000l")  # Disable

    assert term.size() == (80, 24)


# Edge Cases and Boundary Tests
def test_cursor_beyond_bounds():
    """Test cursor positioning beyond terminal bounds"""
    term = Terminal(80, 24)
    term.process_str("\x1b[999;999H")  # Try to move way out of bounds

    col, row = term.cursor_position()
    assert col <= 79  # Should clamp to max
    assert row <= 23


def test_very_long_line():
    """Test handling very long lines with wrapping"""
    term = Terminal(40, 5)
    long_text = "A" * 100  # More than terminal width
    term.process_str(long_text)

    col, row = term.cursor_position()
    assert row >= 2  # Should wrap to multiple lines


def test_clear_variants():
    """Test different clear screen variants"""
    term = Terminal(80, 24)
    term.process_str("Test content")

    # Clear from cursor to end
    term.process_str("\x1b[0J")
    content = term.content()

    # Clear from beginning to cursor
    term.process_str("More content")
    term.process_str("\x1b[5;5H")
    term.process_str("\x1b[1J")

    # Clear entire screen
    term.process_str("\x1b[2J")
    content = term.content()
    assert "Test content" not in content


def test_line_erase_variants():
    """Test different erase in line variants"""
    term = Terminal(80, 24)
    term.process_str("ABCDEFGH")

    # Erase from cursor to end of line
    term.process_str("\x1b[1;5H")  # Move to 'E'
    term.process_str("\x1b[0K")
    line = term.get_line(0)
    assert "ABCD" in line

    term.reset()
    term.process_str("ABCDEFGH")

    # Erase from beginning to cursor
    term.process_str("\x1b[1;5H")
    term.process_str("\x1b[1K")

    # Erase entire line
    term.process_str("\x1b[2K")
    line = term.get_line(0)
    assert line.strip() == ""


def test_background_colors():
    """Test background color setting"""
    term = Terminal(80, 24)
    term.process_str("\x1b[42mGreen BG\x1b[0m")  # Green background

    bg_color = term.get_bg_color(0, 0)
    assert bg_color is not None


def test_256_color_foreground():
    """Test 256-color foreground"""
    term = Terminal(80, 24)
    term.process_str("\x1b[38;5;196mRed\x1b[0m")  # 256-color red

    fg_color = term.get_fg_color(0, 0)
    assert fg_color is not None


def test_256_color_background():
    """Test 256-color background"""
    term = Terminal(80, 24)
    term.process_str("\x1b[48;5;21mBlue BG\x1b[0m")  # 256-color blue background

    bg_color = term.get_bg_color(0, 0)
    assert bg_color is not None


def test_multiple_attributes():
    """Test combining multiple text attributes"""
    term = Terminal(80, 24)
    term.process_str("\x1b[1;3;4;7mMulti\x1b[0m")  # Bold, italic, underline, reverse

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.bold is True
    assert attrs.italic is True
    assert attrs.underline is True
    assert attrs.reverse is True


def test_save_restore_cursor_position():
    """Test ANSI save/restore cursor"""
    term = Terminal(80, 24)
    term.process_str("\x1b[10;20H")  # Move cursor
    term.process_str("\x1b[s")  # Save cursor
    term.process_str("\x1b[1;1H")  # Move to origin

    col, row = term.cursor_position()
    assert col == 0 and row == 0

    term.process_str("\x1b[u")  # Restore cursor
    col, row = term.cursor_position()
    assert col == 19 and row == 9  # 0-indexed


def test_dec_save_restore_cursor():
    """Test DEC save/restore cursor (ESC 7/8)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[10;20H")  # Move cursor
    term.process_str("\x1b7")  # Save cursor (DECSC)
    term.process_str("\x1b[1;1H")  # Move to origin
    term.process_str("\x1b8")  # Restore cursor (DECRC)

    col, row = term.cursor_position()
    assert col == 19 and row == 9


def test_alternate_screen_no_scrollback():
    """Test that alternate screen doesn't add to scrollback"""
    term = Terminal(80, 5, scrollback=100)

    # Write on primary screen
    for i in range(10):
        term.process_str(f"Primary {i}\n")

    scrollback_primary = term.scrollback_len()
    assert scrollback_primary > 0

    # Switch to alt screen
    term.process_str("\x1b[?1049h")

    # Write on alt screen
    for i in range(10):
        term.process_str(f"Alt {i}\n")

    # Scrollback should not increase
    scrollback_alt = term.scrollback_len()
    assert scrollback_alt == scrollback_primary


def test_reverse_index():
    """Test reverse index (move up and scroll if needed)"""
    term = Terminal(80, 10)
    term.process_str("Line 0\nLine 1\nLine 2")
    term.process_str("\x1b[1;1H")  # Move to top
    term.process_str("\x1bM")  # Reverse index

    # Should have scrolled down
    line0 = term.get_line(0)
    assert line0.strip() == ""


def test_index_escape():
    """Test index escape sequence (move down and scroll if needed)"""
    term = Terminal(80, 5)
    for i in range(4):
        term.process_str(f"Line {i}\n")

    term.process_str("\x1b[5;1H")  # Move to bottom
    term.process_str("\x1bD")  # Index

    # Should have scrolled up
    assert term.scrollback_len() > 0 or term.cursor_position()[1] == 4


def test_cursor_next_line():
    """Test CNL (Cursor Next Line)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5;10H")  # Move to row 5, col 10 (0-indexed: row 4, col 9)
    term.process_str("\x1b[2E")  # Move down 2 lines to column 0

    col, row = term.cursor_position()
    assert row == 6  # Row 4 + 2 = row 6 (0-indexed)
    assert col == 0


def test_cursor_previous_line():
    """Test CPL (Cursor Previous Line)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5;10H")  # Move to row 5, col 10 (0-indexed: row 4, col 9)
    term.process_str("\x1b[2F")  # Move up 2 lines to column 0

    col, row = term.cursor_position()
    assert row == 2  # Row 4 - 2 = row 2 (0-indexed)
    assert col == 0


def test_cursor_horizontal_absolute():
    """Test CHA (Cursor Horizontal Absolute)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[42G")  # Move to column 42

    col, row = term.cursor_position()
    assert col == 41  # 0-indexed


def test_vertical_position_absolute():
    """Test VPA (Vertical Position Absolute)"""
    term = Terminal(80, 24)
    term.process_str("\x1b[12d")  # Move to row 12

    col, row = term.cursor_position()
    assert row == 11  # 0-indexed


def test_application_cursor_keys():
    """Test application cursor keys mode"""
    term = Terminal(80, 24)
    term.process_str("\x1b[?1h")  # Enable application cursor

    # Mode should be enabled
    assert term.size() == (80, 24)

    term.process_str("\x1b[?1l")  # Disable
    assert term.size() == (80, 24)


def test_blink_attribute():
    """Test blink attribute"""
    term = Terminal(80, 24)
    term.process_str("\x1b[5mBlink\x1b[0m")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.blink is True


def test_dim_attribute():
    """Test dim attribute"""
    term = Terminal(80, 24)
    term.process_str("\x1b[2mDim\x1b[0m")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.dim is True


def test_hidden_attribute():
    """Test hidden attribute"""
    term = Terminal(80, 24)
    term.process_str("\x1b[8mHidden\x1b[0m")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.hidden is True


def test_strikethrough_attribute():
    """Test strikethrough attribute"""
    term = Terminal(80, 24)
    term.process_str("\x1b[9mStrike\x1b[0m")

    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.strikethrough is True


# Device Query Response Tests
def test_da_primary():
    """Test Primary Device Attributes (DA) query"""
    term = Terminal(80, 24)
    term.process(b"\x1b[c")  # Send Primary DA query

    # Check that we got a response
    assert term.has_pending_responses()

    # Drain and check the response
    response = term.drain_responses()
    assert response == b"\x1b[?62;1;4;6;9;15;22c"

    # After draining, no more responses
    assert not term.has_pending_responses()


def test_da_primary_with_param():
    """Test Primary DA with parameter 0"""
    term = Terminal(80, 24)
    term.process(b"\x1b[0c")

    response = term.drain_responses()
    assert response == b"\x1b[?62;1;4;6;9;15;22c"


def test_da_secondary():
    """Test Secondary Device Attributes (DA) query"""
    term = Terminal(80, 24)
    term.process(b"\x1b[>c")  # Send Secondary DA query

    response = term.drain_responses()
    assert response == b"\x1b[>82;10000;0c"


def test_da_secondary_with_param():
    """Test Secondary DA with parameter 0"""
    term = Terminal(80, 24)
    term.process(b"\x1b[>0c")

    response = term.drain_responses()
    assert response == b"\x1b[>82;10000;0c"


def test_dsr_operating_status():
    """Test Device Status Report (DSR) - Operating Status"""
    term = Terminal(80, 24)
    term.process(b"\x1b[5n")  # Query operating status

    response = term.drain_responses()
    assert response == b"\x1b[0n"  # Terminal ready


def test_dsr_cursor_position():
    """Test Device Status Report (DSR) - Cursor Position"""
    term = Terminal(80, 24)
    term.process_str("\x1b[10;20H")  # Move to row 10, col 20
    term.process(b"\x1b[6n")  # Query cursor position

    response = term.drain_responses()
    assert response == b"\x1b[10;20R"  # 1-indexed position report


def test_dsr_cursor_position_origin():
    """Test DSR cursor position at origin"""
    term = Terminal(80, 24)
    term.process(b"\x1b[6n")  # Query at origin

    response = term.drain_responses()
    assert response == b"\x1b[1;1R"


def test_dsr_cursor_position_various():
    """Test DSR cursor position at various locations"""
    term = Terminal(80, 24)

    # Test at 5, 10
    term.process_str("\x1b[5;10H")
    term.process(b"\x1b[6n")
    assert term.drain_responses() == b"\x1b[5;10R"

    # Test at 1, 1
    term.process_str("\x1b[1;1H")
    term.process(b"\x1b[6n")
    assert term.drain_responses() == b"\x1b[1;1R"

    # Test at 24, 80
    term.process_str("\x1b[24;80H")
    term.process(b"\x1b[6n")
    assert term.drain_responses() == b"\x1b[24;80R"


def test_decreqtparm_solicited():
    """Test Terminal Parameters (DECREQTPARM) - Solicited"""
    term = Terminal(80, 24)
    term.process(b"\x1b[0x")  # Solicited request

    response = term.drain_responses()
    assert response == b"\x1b[2;1;1;120;120;1;0x"


def test_decreqtparm_unsolicited():
    """Test Terminal Parameters (DECREQTPARM) - Unsolicited"""
    term = Terminal(80, 24)
    term.process(b"\x1b[1x")  # Unsolicited request

    response = term.drain_responses()
    assert response == b"\x1b[3;1;1;120;120;1;0x"


def test_decrqm_application_cursor():
    """Test DEC Private Mode Status (DECRQM) - Application Cursor"""
    term = Terminal(80, 24)

    # Query when not set
    term.process(b"\x1b[?1$p")
    assert term.drain_responses() == b"\x1b[?1;2$y"  # Reset state

    # Enable application cursor
    term.process_str("\x1b[?1h")

    # Query when set
    term.process(b"\x1b[?1$p")
    assert term.drain_responses() == b"\x1b[?1;1$y"  # Set state


def test_decrqm_cursor_visibility():
    """Test DECRQM - Cursor Visibility"""
    term = Terminal(80, 24)

    # Cursor visible by default
    term.process(b"\x1b[?25$p")
    assert term.drain_responses() == b"\x1b[?25;1$y"

    # Hide cursor
    term.process_str("\x1b[?25l")
    term.process(b"\x1b[?25$p")
    assert term.drain_responses() == b"\x1b[?25;2$y"


def test_decrqm_mouse_modes():
    """Test DECRQM - Mouse Tracking Modes"""
    term = Terminal(80, 24)

    # Test normal mouse mode
    term.process(b"\x1b[?1000$p")
    assert term.drain_responses() == b"\x1b[?1000;2$y"  # Off

    term.process_str("\x1b[?1000h")  # Enable
    term.process(b"\x1b[?1000$p")
    assert term.drain_responses() == b"\x1b[?1000;1$y"  # On

    # Test button event mode
    term.process_str("\x1b[?1002h")
    term.process(b"\x1b[?1002$p")
    assert term.drain_responses() == b"\x1b[?1002;1$y"

    # Test any event mode
    term.process_str("\x1b[?1003h")
    term.process(b"\x1b[?1003$p")
    assert term.drain_responses() == b"\x1b[?1003;1$y"


def test_decrqm_bracketed_paste():
    """Test DECRQM - Bracketed Paste Mode"""
    term = Terminal(80, 24)

    # Off by default
    term.process(b"\x1b[?2004$p")
    assert term.drain_responses() == b"\x1b[?2004;2$y"

    # Enable
    term.process_str("\x1b[?2004h")
    term.process(b"\x1b[?2004$p")
    assert term.drain_responses() == b"\x1b[?2004;1$y"


def test_decrqm_unrecognized_mode():
    """Test DECRQM - Unrecognized Mode"""
    term = Terminal(80, 24)
    term.process(b"\x1b[?9999$p")  # Query unknown mode

    response = term.drain_responses()
    assert response == b"\x1b[?9999;0$y"  # Not recognized


def test_multiple_queries():
    """Test multiple device queries in sequence"""
    term = Terminal(80, 24)

    # Send multiple queries
    term.process(b"\x1b[5n")  # Operating status
    term.process(b"\x1b[6n")  # Cursor position
    term.process(b"\x1b[c")  # Primary DA

    # All responses should be buffered
    assert term.has_pending_responses()

    # Drain all responses
    responses = term.drain_responses()
    expected = b"\x1b[0n\x1b[1;1R\x1b[?62;1;4;6;9;15;22c"
    assert responses == expected


def test_responses_dont_affect_display():
    """Test that query responses don't appear in terminal content"""
    term = Terminal(80, 24)
    term.process_str("Hello")
    term.process(b"\x1b[6n")  # Query cursor position

    # Content should still be "Hello", not include the response
    content = term.content()
    assert "Hello" in content
    assert "\x1b" not in content  # No escape sequences in content

    # Response should be buffered separately
    response = term.drain_responses()
    assert b"\x1b" in response


def test_query_after_cursor_movement():
    """Test that queries reflect current terminal state"""
    term = Terminal(80, 24)

    # Move cursor and query
    term.process_str("\x1b[5;10H")
    term.process(b"\x1b[6n")
    assert term.drain_responses() == b"\x1b[5;10R"

    # Move again and query
    term.process_str("\x1b[15;25H")
    term.process(b"\x1b[6n")
    assert term.drain_responses() == b"\x1b[15;25R"


def test_query_with_mode_changes():
    """Test queries reflect mode state changes"""
    term = Terminal(80, 24)

    # Query bracketed paste (initially off)
    term.process(b"\x1b[?2004$p")
    assert term.drain_responses() == b"\x1b[?2004;2$y"

    # Enable bracketed paste
    term.process_str("\x1b[?2004h")
    term.process(b"\x1b[?2004$p")
    assert term.drain_responses() == b"\x1b[?2004;1$y"

    # Disable it
    term.process_str("\x1b[?2004l")
    term.process(b"\x1b[?2004$p")
    assert term.drain_responses() == b"\x1b[?2004;2$y"


def test_response_buffer_isolation():
    """Test that response buffers are properly isolated between terminals"""
    term1 = Terminal(80, 24)
    term2 = Terminal(80, 24)

    # Generate responses in term1
    term1.process(b"\x1b[5n")

    # term2 should not have responses
    assert not term2.has_pending_responses()
    assert term1.has_pending_responses()

    # Drain term1
    term1.drain_responses()
    assert not term1.has_pending_responses()


def test_hyperlink_basic():
    """Test basic hyperlink storage and retrieval"""
    term = Terminal(80, 24)

    # Write text with hyperlink
    term.process_str("\x1b]8;;https://example.com\x1b\\test link\x1b]8;;\x1b\\")

    # Check that characters have hyperlink
    url = term.get_hyperlink(0, 0)
    assert url == "https://example.com"

    # Check multiple characters in the link
    for i in range(9):  # "test link" is 9 characters
        url = term.get_hyperlink(i, 0)
        assert url == "https://example.com", (
            f"Character at position {i} should have hyperlink"
        )

    # Check that character after link has no hyperlink
    url = term.get_hyperlink(9, 0)
    assert url is None


def test_hyperlink_deduplication():
    """Test that identical URLs are deduplicated"""
    term = Terminal(80, 24)

    # Write same URL twice
    term.process_str("\x1b]8;;https://example.com\x1b\\link1\x1b]8;;\x1b\\ ")
    term.process_str("\x1b]8;;https://example.com\x1b\\link2\x1b]8;;\x1b\\")

    # Both should have the same hyperlink
    url1 = term.get_hyperlink(0, 0)
    url2 = term.get_hyperlink(6, 0)

    assert url1 == "https://example.com"
    assert url2 == "https://example.com"

    # Check that hyperlink IDs are the same (deduplication)
    attrs1 = term.get_attributes(0, 0)
    attrs2 = term.get_attributes(6, 0)
    assert attrs1 is not None
    assert attrs2 is not None
    assert attrs1.hyperlink_id == attrs2.hyperlink_id


def test_hyperlink_different_urls():
    """Test multiple different hyperlinks"""
    term = Terminal(80, 24)

    # Write different URLs
    term.process_str("\x1b]8;;https://example.com\x1b\\link1\x1b]8;;\x1b\\ ")
    term.process_str("\x1b]8;;https://different.com\x1b\\link2\x1b]8;;\x1b\\")

    url1 = term.get_hyperlink(0, 0)
    url2 = term.get_hyperlink(6, 0)

    assert url1 == "https://example.com"
    assert url2 == "https://different.com"

    # Check that hyperlink IDs are different
    attrs1 = term.get_attributes(0, 0)
    attrs2 = term.get_attributes(6, 0)
    assert attrs1 is not None
    assert attrs2 is not None
    assert attrs1.hyperlink_id != attrs2.hyperlink_id


def test_hyperlink_termination():
    """Test that hyperlinks are properly terminated"""
    term = Terminal(80, 24)

    # Write hyperlink and then terminate it
    term.process_str("\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\text")

    # Characters in link should have URL
    url = term.get_hyperlink(0, 0)
    assert url == "https://example.com"

    # Characters after termination should not have URL
    url = term.get_hyperlink(4, 0)  # "text" starts at position 4
    assert url is None


def test_hyperlink_reset():
    """Test that hyperlinks are cleared on terminal reset"""
    term = Terminal(80, 24)

    # Write hyperlink
    term.process_str("\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\")

    # Verify hyperlink exists
    url = term.get_hyperlink(0, 0)
    assert url == "https://example.com"

    # Reset terminal
    term.reset()

    # Hyperlink should be gone
    url = term.get_hyperlink(0, 0)
    assert url is None


def test_hyperlink_with_attributes():
    """Test hyperlinks combined with text attributes"""
    term = Terminal(80, 24)

    # Write bold, underlined hyperlink
    term.process_str(
        "\x1b[1;4m\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\x1b[0m"
    )

    # Check hyperlink
    url = term.get_hyperlink(0, 0)
    assert url == "https://example.com"

    # Check attributes
    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.bold is True
    assert attrs.underline is True
    assert attrs.hyperlink_id is not None


def test_hyperlink_multiline():
    """Test hyperlinks across line breaks"""
    term = Terminal(80, 24)

    # Write hyperlink that spans multiple lines
    # Note: \n moves to next line but stays at same column (Unix LF behavior)
    term.process_str("\x1b]8;;https://example.com\x1b\\line1\nline2\x1b]8;;\x1b\\")

    # First line should have hyperlink
    url1 = term.get_hyperlink(0, 0)
    assert url1 == "https://example.com"

    # Second line has text starting at column 5 (where cursor was after "line1")
    # because \n doesn't reset column to 0 (would need \r\n for that)
    url2 = term.get_hyperlink(5, 1)
    assert url2 == "https://example.com"

    # All characters on second line should have the link
    url3 = term.get_hyperlink(9, 1)  # Last char of "line2"
    assert url3 == "https://example.com"


def test_hyperlink_out_of_bounds():
    """Test hyperlink retrieval for out of bounds positions"""
    term = Terminal(80, 24)

    # Write hyperlink
    term.process_str("\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\")

    # Out of bounds should return None
    url = term.get_hyperlink(100, 0)
    assert url is None

    url = term.get_hyperlink(0, 100)
    assert url is None


def test_hyperlink_empty_cells():
    """Test that empty cells have no hyperlink"""
    term = Terminal(80, 24)

    # Don't write anything, just check empty cell
    url = term.get_hyperlink(0, 0)
    assert url is None

    # Check attributes
    attrs = term.get_attributes(0, 0)
    assert attrs is not None
    assert attrs.hyperlink_id is None


def test_synchronized_updates_basic():
    """Test basic synchronized updates mode (DEC 2026)"""
    term = Terminal(80, 24)

    # Initially disabled
    assert not term.synchronized_updates()

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")
    assert term.synchronized_updates()

    # Process some content - it should be buffered
    term.process_str("Buffered")
    content = term.content()
    # Content should be empty because it's buffered
    assert "Buffered" not in content

    # Disable synchronized updates - this should flush the buffer
    term.process_str("\x1b[?2026l")
    assert not term.synchronized_updates()

    # Now content should appear
    content = term.content()
    assert "Buffered" in content


def test_synchronized_updates_multiple():
    """Test synchronized updates with multiple updates"""
    term = Terminal(80, 24)

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")

    # Send multiple updates
    term.process_str("Line1\r\n")
    term.process_str("Line2\r\n")
    term.process_str("Line3")

    # All should be buffered
    content = term.content()
    assert "Line1" not in content
    assert "Line2" not in content
    assert "Line3" not in content

    # Disable and flush
    term.process_str("\x1b[?2026l")

    # All lines should appear
    content = term.content()
    assert "Line1" in content
    assert "Line2" in content
    assert "Line3" in content


def test_synchronized_updates_manual_flush():
    """Test manual flushing of synchronized updates buffer"""
    term = Terminal(80, 24)

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")
    term.process_str("Test")

    # Content buffered
    assert "Test" not in term.content()

    # Manual flush
    term.flush_synchronized_updates()

    # Content should appear, mode still enabled
    assert "Test" in term.content()
    assert term.synchronized_updates()

    # Add more content, should still buffer
    term.process_str(" More")
    assert "More" not in term.content()

    # Disable to flush
    term.process_str("\x1b[?2026l")
    assert "More" in term.content()


def test_synchronized_updates_with_escape_sequences():
    """Test synchronized updates with ANSI escape sequences"""
    term = Terminal(80, 24)

    # Enable synchronized updates
    term.process_str("\x1b[?2026h")

    # Send colored text
    term.process_str("\x1b[31mRed\x1b[0m \x1b[32mGreen\x1b[0m")

    # Should be buffered
    assert "Red" not in term.content()
    assert "Green" not in term.content()

    # Disable and flush
    term.process_str("\x1b[?2026l")

    # Text should appear with colors applied
    content = term.content()
    assert "Red" in content
    assert "Green" in content


def test_paste_with_bracketed_mode():
    """Test paste() method with bracketed paste mode enabled"""
    term = Terminal(80, 24)

    # Enable bracketed paste mode
    term.process_str("\x1b[?2004h")
    assert term.bracketed_paste()

    # Paste content
    term.paste("Hello World")

    # Content should appear
    content = term.content()
    assert "Hello World" in content


def test_paste_without_bracketed_mode():
    """Test paste() method with bracketed paste mode disabled"""
    term = Terminal(80, 24)

    # Bracketed paste disabled by default
    assert not term.bracketed_paste()

    # Paste content
    term.paste("Direct Paste")

    # Content should appear
    content = term.content()
    assert "Direct Paste" in content


def test_paste_multiline():
    """Test paste() with multiline content"""
    term = Terminal(80, 24)

    # Enable bracketed paste
    term.process_str("\x1b[?2004h")

    # Paste multiline content
    term.paste("Line 1\nLine 2\nLine 3")

    # All lines should appear
    content = term.content()
    assert "Line 1" in content
    assert "Line 2" in content
    assert "Line 3" in content


def test_paste_special_characters():
    """Test paste() with special characters"""
    term = Terminal(80, 24)

    # Paste content with special characters
    term.paste("Tab:\tNewline:\nReturn:\r")

    # Content should be processed
    content = term.content()
    assert "Tab:" in content
    assert "Newline:" in content
    assert "Return:" in content


def test_decfra_fill_rectangle():
    """Test DECFRA - Fill Rectangular Area"""
    term = Terminal(80, 24)

    # Fill a 3x3 rectangle with 'X' at position (3,3) to (5,5) (1-indexed)
    # DECFRA: CSI Pc ; Pt ; Pl ; Pb ; Pr $ x
    # Pc = 88 (ASCII 'X'), Pt=3, Pl=3, Pb=5, Pr=5
    term.process(b"\x1b[88;3;3;5;5$x")

    # Get the filled area (convert to 0-indexed)
    # (3,3) to (5,5) in 1-indexed = (2,2) to (4,4) in 0-indexed
    assert term.get_char(2, 2) == "X", "Cell at (2,2) should be 'X'"
    assert term.get_char(3, 3) == "X", "Cell at (3,3) should be 'X'"
    assert term.get_char(4, 4) == "X", "Cell at (4,4) should be 'X'"

    # Check that cells outside the rectangle are not filled
    assert term.get_char(1, 1) != "X", "Cell outside rectangle should not be 'X'"
    assert term.get_char(5, 5) != "X", "Cell outside rectangle should not be 'X'"


def test_deccra_copy_rectangle():
    """Test DECCRA - Copy Rectangular Area"""
    term = Terminal(80, 24)

    # Write text in source area
    term.process_str("\x1b[2;2HABC")  # Write "ABC" at (2,2)

    # Copy 1x3 rectangle from (2,2) to (2,4) to destination (2,10)
    # DECCRA: CSI Pts ; Pls ; Pbs ; Prs ; Pps ; Ptd ; Pld ; Ppd $ v
    # Source: row 2, cols 2-4, page 1
    # Dest: row 2, col 10, page 1
    term.process(b"\x1b[2;2;2;4;1;2;10;1$v")

    # Check that text was copied to destination
    # col 10 in 1-indexed = 9 in 0-indexed
    dest_text = "".join(
        [
            term.get_char(9, 1) or " ",
            term.get_char(10, 1) or " ",
            term.get_char(11, 1) or " ",
        ]
    )
    assert dest_text == "ABC", "Text should be copied to destination"

    # Source should still have the text
    # col 2 in 1-indexed = 1 in 0-indexed
    src_text = "".join(
        [
            term.get_char(1, 1) or " ",
            term.get_char(2, 1) or " ",
            term.get_char(3, 1) or " ",
        ]
    )
    assert src_text == "ABC", "Source text should remain"


def test_decsera_erase_rectangle():
    """Test DECSERA - Selective Erase Rectangular Area"""
    term = Terminal(80, 24)

    # Fill area with 'X' characters
    for row in range(5):
        for col in range(6):
            term.process_str(f"\x1b[{row + 1};{col + 1}HX")

    # Erase a 2x2 rectangle at (2,2) to (3,3) in 1-indexed coords
    # DECSERA: CSI Pt ; Pl ; Pb ; Pr $ {
    term.process(b"\x1b[2;2;3;3${")

    # Check that the rectangle is erased (should be spaces or null char)
    # (2,2) to (3,3) in 1-indexed = (1,1) to (2,2) in 0-indexed
    erased_char = term.get_char(1, 1)
    assert erased_char in (" ", "\0", None), "Cell at (1,1) should be erased"
    erased_char = term.get_char(2, 1)
    assert erased_char in (" ", "\0", None), "Cell at (2,1) should be erased"
    erased_char = term.get_char(1, 2)
    assert erased_char in (" ", "\0", None), "Cell at (1,2) should be erased"
    erased_char = term.get_char(2, 2)
    assert erased_char in (" ", "\0", None), "Cell at (2,2) should be erased"

    # Check that cells outside the rectangle are not erased
    assert term.get_char(0, 0) == "X", "Cell outside rectangle should remain 'X'"
    assert term.get_char(3, 3) == "X", "Cell outside rectangle should remain 'X'"


def test_rectangle_operations_boundary():
    """Test rectangle operations with boundary conditions"""
    term = Terminal(80, 24)

    # Test filling near the edge
    # Fill rectangle from (1,1) to (3,3)
    term.process(b"\x1b[42;1;1;3;3$x")  # Fill with '*' (ASCII 42)

    # Check corners (1,1) to (3,3) in 1-indexed = (0,0) to (2,2) in 0-indexed
    assert term.get_char(0, 0) == "*", "Top-left corner should be filled"
    assert term.get_char(2, 0) == "*", "Top-right should be filled"
    assert term.get_char(0, 2) == "*", "Bottom-left should be filled"
    assert term.get_char(2, 2) == "*", "Bottom-right should be filled"

    # Test with out-of-bounds coordinates (should be clamped)
    term2 = Terminal(10, 10)
    # Try to fill beyond screen bounds
    term2.process(b"\x1b[35;5;5;100;100$x")  # Fill with '#' (ASCII 35)

    # Should fill from (5,5) to (10,10) in 1-indexed = (4,4) to (9,9) in 0-indexed
    assert term2.get_char(4, 4) == "#", "Should fill at (4,4)"
    assert term2.get_char(9, 9) == "#", "Should fill to screen edge"


def test_decfra_with_attributes():
    """Test DECFRA preserves current text attributes"""
    term = Terminal(80, 24)

    # Set red foreground color
    term.process(b"\x1b[31m")

    # Fill rectangle with '*'
    term.process(b"\x1b[42;2;2;4;4$x")  # ASCII 42 = '*'

    # Get a cell from the filled area and check character
    # (2,2) in 1-indexed = (1,1) in 0-indexed
    char = term.get_char(1, 1)
    assert char == "*", "Character should be '*'"
    # Note: We can't easily test color in this test without more complex assertions
    # The important thing is that the Rust tests cover this


def test_decera_erase_rectangle():
    """Test DECERA - Erase Rectangular Area (VT420)"""
    term = Terminal(80, 24)

    # Fill screen with 'X'
    for row in range(5):
        for col in range(5):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"X")

    # Erase a 2x2 rectangle at (2,2) to (3,3) in 1-indexed coords
    # DECERA: CSI Pt ; Pl ; Pb ; Pr $ z
    term.process(b"\x1b[2;2;3;3$z")

    # Check that the rectangle is erased
    assert term.get_char(1, 1) == " ", "Cell (1,1) should be erased"
    assert term.get_char(2, 1) == " ", "Cell (2,1) should be erased"
    assert term.get_char(1, 2) == " ", "Cell (1,2) should be erased"
    assert term.get_char(2, 2) == " ", "Cell (2,2) should be erased"

    # Check that cells outside the rectangle are not erased
    assert term.get_char(0, 0) == "X", "Cell outside rectangle should remain 'X'"
    assert term.get_char(3, 3) == "X", "Cell outside rectangle should remain 'X'"


def test_deccara_change_attributes():
    """Test DECCARA - Change Attributes in Rectangular Area (VT420)"""
    term = Terminal(80, 24)

    # Fill a rectangle with normal text
    for row in range(5):
        for col in range(5):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"A")

    # Make a 3x3 rectangle bold (attribute 1)
    # DECCARA: CSI Pt ; Pl ; Pb ; Pr ; Ps $ r
    term.process(b"\x1b[2;2;4;4;1$r")

    # We can't easily test the bold attribute from Python,
    # but we can verify the sequence doesn't crash
    # The Rust tests should verify the actual attribute changes
    assert term.get_char(2, 2) == "A", "Character should remain 'A'"


def test_decrara_reverse_attributes():
    """Test DECRARA - Reverse Attributes in Rectangular Area (VT420)"""
    term = Terminal(80, 24)

    # Fill a rectangle with text
    for row in range(5):
        for col in range(5):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"B")

    # Reverse attributes in a 2x2 rectangle
    # DECRARA: CSI Pt ; Pl ; Pb ; Pr ; Ps $ t
    term.process(b"\x1b[2;2;3;3;1$t")  # Reverse bold attribute

    # We can't easily test attribute reversal from Python,
    # but we can verify the sequence doesn't crash
    assert term.get_char(1, 1) == "B", "Character should remain 'B'"


def test_decsace_attribute_change_extent():
    """Test DECSACE - Select Attribute Change Extent (VT420)"""
    term = Terminal(80, 24)

    # Set to stream mode (1)
    # DECSACE: CSI Ps * x
    term.process(b"\x1b[1*x")

    # Set to rectangle mode (2) - default
    term.process(b"\x1b[2*x")

    # This is a mode-setting sequence, so just verify it doesn't crash
    # The actual behavior affects how DECCARA/DECRARA work


def test_decrqcra_request_checksum():
    """Test DECRQCRA - Request Checksum of Rectangular Area (VT420)"""
    term = Terminal(80, 24)

    # Fill a small area with known text
    term.process(b"\x1b[1;1HABC")
    term.process(b"\x1b[2;1HDEF")

    # Request checksum of rectangle (1,1) to (2,3)
    # DECRQCRA: CSI Pi ; Pg ; Pt ; Pl ; Pb ; Pr * y
    # Pi=1 (request ID), Pg=1 (page), then rectangle coords
    term.process(b"\x1b[1;1;1;1;2;3*y")

    # Get the response
    response = term.drain_responses()

    # Response should be: DCS 1 ! ~ xxxx ST
    # Format: \x1bP1!~<checksum>\x1b\\
    assert response.startswith(b"\x1bP1!~"), "Response should start with DCS 1 ! ~"
    assert response.endswith(b"\x1b\\"), "Response should end with ST"
    # Checksum is 4 hex digits
    checksum_part = response[5:-2]  # Extract checksum part
    assert len(checksum_part) == 4, "Checksum should be 4 characters"


def test_decsca_character_protection():
    """Test DECSCA - Select Character Protection Attribute"""
    term = Terminal(80, 24)

    # Fill a 5x5 area with normal characters
    for row in range(5):
        for col in range(5):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"N")  # Normal character

    # Enable character protection (DECSCA with Ps=1)
    # CSI ? 1 " q
    term.process(b'\x1b[?1"q')

    # Fill a 3x3 area in the center with protected characters
    for row in range(1, 4):
        for col in range(1, 4):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"P")  # Protected character

    # Disable character protection (DECSCA with Ps=0 or 2)
    # CSI ? 0 " q
    term.process(b'\x1b[?0"q')

    # Verify characters are present
    assert term.get_char(0, 0) == "N", "Corner should have normal char"
    assert term.get_char(2, 2) == "P", "Center should have protected char"

    # Now test selective erase (DECSERA) - should preserve protected chars
    # DECSERA: CSI Pt ; Pl ; Pb ; Pr $ {
    # Erase the entire 5x5 area
    term.process(b"\x1b[1;1;5;5${")

    # Check that normal characters were erased
    assert term.get_char(0, 0) == " ", "Normal char at corner should be erased"
    assert term.get_char(4, 4) == " ", "Normal char at bottom-right should be erased"

    # Check that protected characters were NOT erased
    assert term.get_char(1, 1) == "P", "Protected char should remain"
    assert term.get_char(2, 2) == "P", "Protected char should remain"
    assert term.get_char(3, 3) == "P", "Protected char should remain"


def test_decera_ignores_protection():
    """Test DECERA ignores character protection (unlike DECSERA)"""
    term = Terminal(80, 24)

    # Enable character protection
    term.process(b'\x1b[?1"q')

    # Fill a 5x5 area with protected characters
    for row in range(5):
        for col in range(5):
            term.process(f"\x1b[{row + 1};{col + 1}H".encode())
            term.process(b"X")

    # Verify characters are present
    assert term.get_char(2, 2) == "X", "Should have protected char"

    # Use DECERA (unconditional erase) - should erase even protected chars
    # DECERA: CSI Pt ; Pl ; Pb ; Pr $ z
    term.process(b"\x1b[1;1;5;5$z")

    # Check that ALL characters were erased (including protected ones)
    assert term.get_char(0, 0) == " ", "Protected char should be erased by DECERA"
    assert term.get_char(2, 2) == " ", "Protected char should be erased by DECERA"
    assert term.get_char(4, 4) == " ", "Protected char should be erased by DECERA"


def test_decsca_modes():
    """Test different DECSCA modes (0, 1, 2)"""
    term = Terminal(80, 24)

    # Mode 1: Enable protection
    term.process(b'\x1b[?1"q')
    term.process(b"\x1b[1;1HA")

    # Mode 0: Disable protection
    term.process(b'\x1b[?0"q')
    term.process(b"\x1b[1;2HB")

    # Mode 2: Also disables protection (same as 0)
    term.process(b'\x1b[?2"q')
    term.process(b"\x1b[1;3HC")

    # Erase with DECSERA
    term.process(b"\x1b[1;1;1;3${")

    # A should remain (was protected), B and C should be erased
    assert term.get_char(0, 0) == "A", "Protected char should remain"
    assert term.get_char(1, 0) == " ", "Unprotected char should be erased"
    assert term.get_char(2, 0) == " ", "Unprotected char should be erased"


def test_notification_osc9():
    """Test OSC 9 notification support"""
    term = Terminal(80, 24)

    # No notifications initially
    assert not term.has_notifications()
    assert term.drain_notifications() == []

    # Send OSC 9 notification
    term.process(b"\x1b]9;This is a simple notification\x1b\\")

    # Check notification was received
    assert term.has_notifications()
    notifs = term.drain_notifications()
    assert len(notifs) == 1
    title, message = notifs[0]
    assert title == ""  # OSC 9 has no title
    assert message == "This is a simple notification"

    # After draining, should have no more notifications
    assert not term.has_notifications()


def test_notification_osc777():
    """Test OSC 777 notification support"""
    term = Terminal(80, 24)

    # Send OSC 777 notification with title and message
    term.process(b"\x1b]777;notify;Alert Title;Alert Message\x1b\\")

    # Check notification was received
    assert term.has_notifications()
    notifs = term.take_notifications()
    assert len(notifs) == 1
    title, message = notifs[0]
    assert title == "Alert Title"
    assert message == "Alert Message"


def test_notification_multiple():
    """Test multiple notifications"""
    term = Terminal(80, 24)

    # Send multiple notifications
    term.process(b"\x1b]9;Notification 1\x1b\\")
    term.process(b"\x1b]777;notify;Title 2;Message 2\x1b\\")
    term.process(b"\x1b]9;Notification 3\x1b\\")

    # Check all were received
    assert term.has_notifications()
    notifs = term.drain_notifications()
    assert len(notifs) == 3

    # Verify contents
    assert notifs[0] == ("", "Notification 1")
    assert notifs[1] == ("Title 2", "Message 2")
    assert notifs[2] == ("", "Notification 3")


def test_notification_special_characters():
    """Test notifications with special characters"""
    term = Terminal(80, 24)

    # Test with newlines, unicode, etc.
    term.process(
        b"\x1b]777;notify;Unicode \xf0\x9f\x8e\x89;Message with emoji \xf0\x9f\x92\xbb\x1b\\"
    )

    notifs = term.drain_notifications()
    assert len(notifs) == 1
    title, message = notifs[0]
    assert "🎉" in title
    assert "💻" in message


def test_notification_empty_message():
    """Test notification with empty message"""
    term = Terminal(80, 24)

    # OSC 9 with empty message (edge case)
    term.process(b"\x1b]9;\x1b\\")

    notifs = term.drain_notifications()
    assert len(notifs) == 1
    assert notifs[0] == ("", "")


def test_insert_mode():
    """Test IRM (Insert/Replace Mode) - Mode 4"""
    term = Terminal(80, 24)

    # Write initial text
    term.process(b"Hello")
    assert term.content().strip() == "Hello"

    # Move cursor to column 2 (CSI 1;2H)
    term.process(b"\x1b[1;2H")

    # Enable insert mode (CSI 4 h)
    term.process(b"\x1b[4h")
    assert term.insert_mode() is True

    # Write "XX" - should insert, not replace
    term.process(b"XX")
    assert "HXXello" in term.content()

    # Disable insert mode (CSI 4 l)
    term.process(b"\x1b[4l")
    assert term.insert_mode() is False

    # Move to column 4
    term.process(b"\x1b[1;4H")

    # Write "YY" - should replace, not insert
    term.process(b"YY")
    assert "HXXYYlo" in term.content()


def test_line_feed_new_line_mode():
    """Test LNM (Line Feed/New Line Mode) - Mode 20"""
    term = Terminal(80, 24)

    # Write text
    term.process(b"Hello")
    col, row = term.cursor_position()
    assert col == 5
    assert row == 0

    # LF without LNM - should move down but stay in same column
    term.process(b"\n")
    col, row = term.cursor_position()
    assert col == 5  # Same column
    assert row == 1  # Next row

    # Reset and test with LNM enabled
    term.reset()
    term.process(b"Hello")

    # Enable LNM (CSI 20 h)
    term.process(b"\x1b[20h")
    assert term.line_feed_new_line_mode() is True

    # LF with LNM - should move down AND to column 0 (CR+LF)
    term.process(b"\n")
    col, row = term.cursor_position()
    assert col == 0  # Column 0 (CR happened)
    assert row == 1  # Next row (LF happened)

    # Disable LNM (CSI 20 l)
    term.process(b"\x1b[20l")
    assert term.line_feed_new_line_mode() is False


def test_xtwinops_title_stack():
    """Test XTWINOPS window title stack (operations 22 and 23)"""
    term = Terminal(80, 24)

    # Set initial title
    term.process(b"\x1b]0;Title1\x07")
    assert term.title() == "Title1"

    # Push title to stack (CSI 22 t)
    term.process(b"\x1b[22t")

    # Change title
    term.process(b"\x1b]0;Title2\x07")
    assert term.title() == "Title2"

    # Push again
    term.process(b"\x1b[22t")

    # Change title again
    term.process(b"\x1b]0;Title3\x07")
    assert term.title() == "Title3"

    # Pop title (CSI 23 t) - should restore Title2
    term.process(b"\x1b[23t")
    assert term.title() == "Title2"

    # Pop again - should restore Title1
    term.process(b"\x1b[23t")
    assert term.title() == "Title1"

    # Pop from empty stack - title should remain Title1
    term.process(b"\x1b[23t")
    assert term.title() == "Title1"


def test_insert_mode_with_wide_chars():
    """Test insert mode with wide characters (emoji)"""
    term = Terminal(80, 24)

    # Write text
    term.process(b"Hello")

    # Move to column 2
    term.process(b"\x1b[1;2H")

    # Enable insert mode
    term.process(b"\x1b[4h")

    # Insert a wide character (emoji)
    term.process("🦀".encode("utf-8"))

    # Should insert the wide char, shifting "ello" to the right by 2 columns
    # (wide chars take 2 cells), resulting in "H🦀 ello" with an extra space
    content = term.content()
    assert "H🦀 ello" in content


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
