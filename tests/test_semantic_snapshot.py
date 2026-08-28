"""Integration tests for the semantic snapshot API."""

import json

from par_term_emu_core_rust import Terminal


def test_visible_snapshot_basic():
    """Visible snapshot returns current screen content."""
    term = Terminal(80, 24)
    term.process(b"Hello, World!\r\n")
    term.process(b"Second line")

    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["cols"] == 80
    assert snap["rows"] == 24
    assert snap["alt_screen_active"] is False
    assert "Hello, World!" in snap["visible_text"]
    assert "Second line" in snap["visible_text"]
    assert snap.get("scrollback_text") is None
    assert snap.get("commands", []) == []
    assert snap["total_commands"] == 0
    assert isinstance(snap["timestamp"], int)
    assert snap["timestamp"] > 0


def test_visible_snapshot_no_history():
    """Visible scope excludes command history and CWD history."""
    term = Terminal(80, 24)
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap.get("commands", []) == []
    assert snap.get("cwd_history", []) == []


def test_full_snapshot_includes_all():
    """Full scope includes all available data."""
    term = Terminal(80, 24)
    term.process(b"Some content\r\n")
    snap = term.get_semantic_snapshot(scope="full")
    assert snap["cols"] == 80
    assert snap["rows"] == 24
    assert "Some content" in snap["visible_text"]


def test_recent_snapshot_with_commands():
    """Recent scope includes only last N commands."""
    term = Terminal(80, 24)
    term.set_accept_osc7(True)

    # Simulate 3 commands with shell integration zones AND command history tracking.
    # OSC 133 sequences handle zone tracking, but command history requires
    # explicit start_command_execution/end_command_execution calls.
    for i in range(3):
        cmd_name = f"cmd{i}"
        term.process(b"\x1b]133;A\x07")  # prompt start
        term.process(b"$ ")
        term.process(b"\x1b]133;B\x07")  # command start
        term.start_command_execution(cmd_name)
        term.process(f"{cmd_name}\r\n".encode())
        term.process(b"\x1b]133;C\x07")  # command executed
        term.process(f"output{i}\r\n".encode())
        term.process(b"\x1b]133;D;0\x07")  # command finished
        term.end_command_execution(0)

    snap = term.get_semantic_snapshot(scope="recent", max_commands=1)
    assert len(snap.get("commands", [])) <= 1
    assert snap["total_commands"] == 3


def test_snapshot_json_string():
    """JSON string output is valid JSON matching dict output."""
    term = Terminal(80, 24)
    term.process(b"Test content")

    json_str = term.get_semantic_snapshot_json(scope="visible")
    parsed = json.loads(json_str)
    assert parsed["cols"] == 80
    assert "Test content" in parsed["visible_text"]


def test_snapshot_json_matches_dict():
    """JSON string and dict output contain the same data."""
    term = Terminal(80, 24)
    term.process(b"Compare me\r\n")

    snap_dict = term.get_semantic_snapshot(scope="visible")
    snap_json = term.get_semantic_snapshot_json(scope="visible")
    parsed = json.loads(snap_json)

    assert snap_dict["cols"] == parsed["cols"]
    assert snap_dict["rows"] == parsed["rows"]
    assert snap_dict["visible_text"] == parsed["visible_text"]


def test_snapshot_with_title():
    """Snapshot captures terminal title."""
    term = Terminal(80, 24)
    term.process(b"\x1b]0;My Title\x07")
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["title"] == "My Title"


def test_snapshot_cursor_position():
    """Snapshot captures cursor position."""
    term = Terminal(80, 24)
    term.process(b"Hello")  # Cursor at col 5, row 0
    snap = term.get_semantic_snapshot(scope="visible")
    assert snap["cursor_col"] == 5
    assert snap["cursor_row"] == 0


def test_snapshot_invalid_scope():
    """Invalid scope raises ValueError."""
    term = Terminal(80, 24)
    try:
        term.get_semantic_snapshot(scope="invalid")
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "scope must be" in str(e)


def test_snapshot_json_invalid_scope():
    """Invalid scope in JSON method raises ValueError."""
    term = Terminal(80, 24)
    try:
        term.get_semantic_snapshot_json(scope="invalid")
        assert False, "Should have raised ValueError"
    except ValueError as e:
        assert "scope must be" in str(e)


def test_snapshot_alt_screen():
    """Snapshot reflects alternate screen state."""
    term = Terminal(80, 24)
    term.process(b"Primary content")
    snap_primary = term.get_semantic_snapshot(scope="visible")
    assert snap_primary["alt_screen_active"] is False

    # Switch to alt screen
    term.process(b"\x1b[?1049h")
    snap_alt = term.get_semantic_snapshot(scope="visible")
    assert snap_alt["alt_screen_active"] is True


def test_snapshot_default_params():
    """Calling with no arguments uses defaults (visible scope, max_commands=10)."""
    term = Terminal(80, 24)
    term.process(b"Default test")
    snap = term.get_semantic_snapshot()
    assert snap["cols"] == 80
    assert "Default test" in snap["visible_text"]


def test_snapshot_json_default_params():
    """JSON method with no arguments uses defaults."""
    term = Terminal(80, 24)
    json_str = term.get_semantic_snapshot_json()
    parsed = json.loads(json_str)
    assert parsed["cols"] == 80


def test_diff_snapshots():
    """diff_snapshots compares the visible text of two snapshot dicts."""
    term = Terminal(80, 24)
    term.process(b"alpha\r\nbeta\r\ngamma")
    old_snap = term.get_semantic_snapshot(scope="visible")

    term.process(b"\x1b[2;1HBETA")  # overwrite row 2 in place
    term.process(b"\x1b[4;1Hdelta")  # new content on row 4
    new_snap = term.get_semantic_snapshot(scope="visible")

    diff = term.diff_snapshots(old_snap, new_snap)
    assert repr(diff).startswith("SnapshotDiff(")

    # content() pads blank rows, so the empty row gaining "delta" is a
    # modified row alongside the beta -> BETA overwrite
    assert diff.modified == 2
    modified = [d for d in diff.diffs if d.change_type == "modified"]
    assert len(modified) == 2

    beta_row = next(d for d in modified if d.old_content == "beta")
    assert beta_row.new_content == "BETA"
    assert beta_row.old_row == 1
    assert beta_row.new_row == 1

    delta_row = next(d for d in modified if d.new_content == "delta")
    assert delta_row.old_content == ""
    assert delta_row.new_row == 3


def test_diff_snapshots_identical():
    """Diffing a snapshot against itself reports only unchanged lines."""
    term = Terminal(80, 24)
    term.process(b"steady state")
    snap = term.get_semantic_snapshot(scope="visible")

    diff = term.diff_snapshots(snap, snap)
    assert diff.added == 0
    assert diff.removed == 0
    assert diff.modified == 0
    assert diff.unchanged == len(diff.diffs)
    assert diff.diffs  # at least the written line
    assert all(d.change_type == "unchanged" for d in diff.diffs)


def test_diff_snapshots_missing_visible_text():
    """A snapshot dict without a visible_text string raises ValueError."""
    import pytest

    term = Terminal(80, 24)
    with pytest.raises(ValueError, match="visible_text"):
        term.diff_snapshots({}, {})
