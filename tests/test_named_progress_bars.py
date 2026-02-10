#!/usr/bin/env python3
"""Tests for OSC 934 named progress bar support."""

from par_term_emu_core_rust import Terminal


def _osc934(action: str, bar_id: str = "", **kwargs: str) -> str:
    """Build an OSC 934 escape sequence."""
    parts = [f"\x1b]934;{action}"]
    if bar_id:
        parts.append(bar_id)
    for key, value in kwargs.items():
        parts.append(f"{key}={value}")
    return ";".join(parts) + "\x1b\\"


def test_set_named_progress_bar():
    """Test creating a named progress bar via OSC 934."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "dl-1", percent="50", label="Downloading"))

    bars = term.named_progress_bars()
    assert "dl-1" in bars
    bar = bars["dl-1"]
    assert bar["id"] == "dl-1"
    assert bar["state"] == "normal"
    assert bar["percent"] == "50"
    assert bar["label"] == "Downloading"


def test_get_named_progress_bar():
    """Test getting a specific named progress bar."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "dl-1", percent="42"))

    bar = term.get_named_progress_bar("dl-1")
    assert bar is not None
    assert bar["percent"] == "42"

    assert term.get_named_progress_bar("nonexistent") is None


def test_update_named_progress_bar():
    """Test updating a progress bar replaces it."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "dl-1", percent="10", label="Starting"))
    term.process_str(_osc934("set", "dl-1", percent="75", label="Almost done"))

    bar = term.get_named_progress_bar("dl-1")
    assert bar is not None
    assert bar["percent"] == "75"
    assert bar["label"] == "Almost done"
    assert len(term.named_progress_bars()) == 1


def test_multiple_progress_bars():
    """Test managing multiple concurrent progress bars."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "a", percent="10"))
    term.process_str(_osc934("set", "b", percent="50"))
    term.process_str(_osc934("set", "c", state="indeterminate"))

    bars = term.named_progress_bars()
    assert len(bars) == 3
    assert bars["a"]["percent"] == "10"
    assert bars["b"]["percent"] == "50"
    assert bars["c"]["state"] == "indeterminate"


def test_remove_named_progress_bar():
    """Test removing a specific progress bar."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "a", percent="10"))
    term.process_str(_osc934("set", "b", percent="20"))

    term.process_str(_osc934("remove", "a"))

    bars = term.named_progress_bars()
    assert len(bars) == 1
    assert "a" not in bars
    assert "b" in bars


def test_remove_all_named_progress_bars():
    """Test removing all progress bars."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "a", percent="10"))
    term.process_str(_osc934("set", "b", percent="20"))

    term.process_str(_osc934("remove_all"))

    assert len(term.named_progress_bars()) == 0


def test_progress_bar_changed_event_on_set():
    """Test that ProgressBarChanged event is emitted on set."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "dl-1", percent="42", label="Test"))

    events = term.poll_events()
    pb_events = [e for e in events if e.get("type") == "progress_bar_changed"]
    assert len(pb_events) == 1
    assert pb_events[0]["action"] == "set"
    assert pb_events[0]["id"] == "dl-1"
    assert pb_events[0]["state"] == "normal"
    assert pb_events[0]["percent"] == "42"
    assert pb_events[0]["label"] == "Test"


def test_progress_bar_changed_event_on_remove():
    """Test that ProgressBarChanged event is emitted on remove."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "dl-1", percent="50"))
    term.poll_events()  # drain

    term.process_str(_osc934("remove", "dl-1"))
    events = term.poll_events()
    pb_events = [e for e in events if e.get("type") == "progress_bar_changed"]
    assert len(pb_events) == 1
    assert pb_events[0]["action"] == "remove"
    assert pb_events[0]["id"] == "dl-1"


def test_progress_bar_changed_event_on_remove_all():
    """Test that ProgressBarChanged event is emitted on remove_all."""
    term = Terminal(80, 24)
    term.process_str(_osc934("set", "a", percent="10"))
    term.process_str(_osc934("set", "b", percent="20"))
    term.poll_events()  # drain

    term.process_str(_osc934("remove_all"))
    events = term.poll_events()
    pb_events = [e for e in events if e.get("type") == "progress_bar_changed"]
    assert len(pb_events) == 1
    assert pb_events[0]["action"] == "remove_all"


def test_event_subscription():
    """Test event subscription filtering for progress_bar_changed."""
    term = Terminal(80, 24)
    term.set_event_subscription(["progress_bar_changed"])

    term.process_str(_osc934("set", "dl-1", percent="50"))

    events = term.poll_subscribed_events()
    assert len(events) == 1
    assert events[0]["type"] == "progress_bar_changed"


def test_manual_set_named_progress_bar():
    """Test manually setting a named progress bar via Python API."""
    term = Terminal(80, 24)
    term.set_named_progress_bar("test-1", state="warning", percent=80, label="Disk low")

    bar = term.get_named_progress_bar("test-1")
    assert bar is not None
    assert bar["state"] == "warning"
    assert bar["percent"] == "80"
    assert bar["label"] == "Disk low"


def test_manual_remove_named_progress_bar():
    """Test manually removing a named progress bar."""
    term = Terminal(80, 24)
    term.set_named_progress_bar("test-1")

    assert term.remove_named_progress_bar("test-1") is True
    assert term.remove_named_progress_bar("nonexistent") is False
    assert len(term.named_progress_bars()) == 0


def test_manual_remove_all():
    """Test manually removing all named progress bars."""
    term = Terminal(80, 24)
    term.set_named_progress_bar("a")
    term.set_named_progress_bar("b")

    term.remove_all_named_progress_bars()
    assert len(term.named_progress_bars()) == 0


def test_osc934_independent_from_osc94():
    """Test that OSC 934 does not affect OSC 9;4 progress bar."""
    term = Terminal(80, 24)

    # Set OSC 9;4 progress
    term.process_str("\x1b]9;4;1;50\x1b\\")
    assert term.has_progress()
    assert term.progress_value() == 50

    # Set OSC 934 bar
    term.process_str(_osc934("set", "dl-1", percent="75"))

    # Both are independent
    assert term.has_progress()
    assert term.progress_value() == 50
    assert term.get_named_progress_bar("dl-1")["percent"] == "75"


def test_state_values():
    """Test all state values are parsed correctly."""
    term = Terminal(80, 24)

    for state_name in ["normal", "indeterminate", "warning", "error"]:
        term.process_str(_osc934("set", f"bar-{state_name}", state=state_name))
        bar = term.get_named_progress_bar(f"bar-{state_name}")
        assert bar is not None
        assert bar["state"] == state_name
