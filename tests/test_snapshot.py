"""Tests for atomic screen snapshot API."""

from par_term_emu_core_rust import Terminal


def test_snapshot_captures_state():
    """Snapshot captures current terminal state."""
    term = Terminal(80, 24)
    term.process(b"Hello, World!\n")

    snapshot = term.create_snapshot()

    assert snapshot.generation >= 0
    assert snapshot.size == (80, 24)
    assert not snapshot.is_alt_screen
    assert len(snapshot.lines) == 24

    # Check first line has content
    line0 = snapshot.get_line(0)
    assert len(line0) == 80
    assert line0[0][0] == "H"  # First char is 'H'


def test_snapshot_is_immutable():
    """Snapshot doesn't change when terminal changes."""
    term = Terminal(80, 24)
    term.process(b"ABC")  # Write at column 0

    snapshot1 = term.create_snapshot()
    gen1 = snapshot1.generation

    # Snapshot1 should show "ABC" at line 0
    line0_snap1 = snapshot1.get_line(0)
    assert line0_snap1[0][0] == "A"
    assert line0_snap1[1][0] == "B"
    assert line0_snap1[2][0] == "C"

    # Modify terminal - overwrite with "XYZ"
    term.process(b"\r")  # Carriage return to go back to column 0
    term.process(b"XYZ")

    # Snapshot1 should still show old state (immutable)
    assert snapshot1.generation == gen1
    line0_still = snapshot1.get_line(0)
    assert line0_still[0][0] == "A"  # Still 'A' from original
    assert line0_still[1][0] == "B"
    assert line0_still[2][0] == "C"

    # New snapshot shows new state
    snapshot2 = term.create_snapshot()
    line0_snap2 = snapshot2.get_line(0)
    assert line0_snap2[0][0] == "X"  # Now shows 'XYZ'
    assert line0_snap2[1][0] == "Y"
    assert line0_snap2[2][0] == "Z"


def test_snapshot_survives_screen_switch():
    """Snapshot remains valid after alternate screen switch."""
    term = Terminal(80, 24)
    term.process(b"Primary")

    # Capture primary screen
    snapshot_primary = term.create_snapshot()
    assert not snapshot_primary.is_alt_screen
    assert snapshot_primary.get_line(0)[0][0] == "P"

    # Switch to alternate screen and reset cursor
    term.process(b"\x1b[?1049h")
    term.process(b"\x1b[H")  # Move cursor to home (0,0)
    term.process(b"Alternate")

    # Old snapshot still valid and shows primary (immutable)
    assert not snapshot_primary.is_alt_screen
    line0_prim = snapshot_primary.get_line(0)
    assert line0_prim[0][0] == "P"  # Still shows "Primary"

    # New snapshot shows alternate screen with new content
    snapshot_alt = term.create_snapshot()
    assert snapshot_alt.is_alt_screen
    line0_alt = snapshot_alt.get_line(0)
    assert line0_alt[0][0] == "A"  # Shows "Alternate" on alt screen


def test_snapshot_cursor_state():
    """Snapshot captures cursor position and visibility."""
    term = Terminal(80, 24)
    term.process(b"Hello")

    snapshot = term.create_snapshot()

    # Cursor should be at column 5 (after "Hello")
    assert snapshot.cursor_pos[0] == 5
    assert snapshot.cursor_pos[1] == 0  # Row 0
    assert snapshot.cursor_visible

    # Hide cursor
    term.process(b"\x1b[?25l")
    snapshot2 = term.create_snapshot()
    assert not snapshot2.cursor_visible


def test_snapshot_out_of_bounds():
    """Snapshot get_line handles out of bounds gracefully."""
    term = Terminal(80, 24)
    snapshot = term.create_snapshot()

    # Valid line
    line0 = snapshot.get_line(0)
    assert len(line0) == 80

    # Out of bounds - should return empty list
    line_invalid = snapshot.get_line(100)
    assert len(line_invalid) == 0


def test_snapshot_with_colors():
    """Snapshot captures cell colors correctly."""
    term = Terminal(80, 24)
    # Set red foreground (SGR 31) and write text
    term.process(b"\x1b[31mRed Text\x1b[0m")

    snapshot = term.create_snapshot()
    line0 = snapshot.get_line(0)

    # First cell should be 'R' with red foreground
    char, fg_rgb, bg_rgb, attrs = line0[0]
    assert char == "R"
    # Red color should be set (not default black)
    assert fg_rgb != (0, 0, 0)


def test_snapshot_with_attributes():
    """Snapshot captures cell attributes correctly."""
    term = Terminal(80, 24)
    # Bold (SGR 1), underline (SGR 4)
    term.process(b"\x1b[1;4mBold+Under\x1b[0m")

    snapshot = term.create_snapshot()
    line0 = snapshot.get_line(0)

    # First cell should have bold and underline
    char, fg_rgb, bg_rgb, attrs = line0[0]
    assert char == "B"
    assert attrs.bold
    assert attrs.underline


def test_snapshot_repr():
    """Snapshot has useful string representation."""
    term = Terminal(80, 24)
    snapshot = term.create_snapshot()

    repr_str = repr(snapshot)
    assert "ScreenSnapshot" in repr_str
    assert "80x24" in repr_str or "size" in repr_str
