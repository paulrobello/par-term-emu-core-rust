#!/usr/bin/env python3
"""Tests for triggers and automation (Feature 18)."""

import pytest
from par_term_emu_core_rust import (
    Terminal,
    TriggerAction,
    TriggerMatch,
    Trigger,
    CoprocessConfig,
)


def test_trigger_crud():
    """Test add, list, get, remove triggers."""
    term = Terminal(80, 24)

    # Add trigger
    action = TriggerAction("highlight", {"bg_r": "255", "bg_g": "0", "bg_b": "0"})
    trigger_id = term.add_trigger("errors", r"ERROR", [action])
    assert isinstance(trigger_id, int)
    assert trigger_id > 0

    # List triggers
    triggers = term.list_triggers()
    assert len(triggers) == 1
    assert isinstance(triggers[0], Trigger)
    assert triggers[0].id == trigger_id
    assert triggers[0].name == "errors"
    assert triggers[0].pattern == "ERROR"
    assert triggers[0].enabled is True

    # Get trigger
    t = term.get_trigger(trigger_id)
    assert t is not None
    assert t.id == trigger_id

    # Get nonexistent
    assert term.get_trigger(99999) is None

    # Remove trigger
    assert term.remove_trigger(trigger_id) is True
    assert term.remove_trigger(trigger_id) is False  # already removed
    assert len(term.list_triggers()) == 0


def test_trigger_match_detection():
    """Test that triggers detect patterns in terminal output."""
    term = Terminal(80, 24)
    term.add_trigger("error", r"ERROR:\s+(\S+)", [])

    term.process_str("prefix ERROR: something_went_wrong\n")
    term.process_trigger_scans()

    matches = term.poll_trigger_matches()
    assert len(matches) == 1
    assert isinstance(matches[0], TriggerMatch)
    assert matches[0].row == 0
    assert "ERROR:" in matches[0].text
    assert len(matches[0].captures) == 2  # group 0 + group 1
    assert matches[0].captures[1] == "something_went_wrong"

    # Subsequent poll should be empty
    assert len(term.poll_trigger_matches()) == 0


def test_trigger_action_types():
    """Test constructing all action types from Python."""
    # Highlight
    a1 = TriggerAction(
        "highlight", {"bg_r": "255", "bg_g": "0", "bg_b": "0", "duration_ms": "5000"}
    )
    assert a1.action_type == "highlight"

    # Notify
    a2 = TriggerAction("notify", {"title": "Alert", "message": "Error: $1"})
    assert a2.action_type == "notify"

    # Mark line
    a3 = TriggerAction("mark_line", {"label": "Important"})
    assert a3.action_type == "mark_line"

    # Set variable
    a4 = TriggerAction("set_variable", {"name": "status", "value": "$1"})
    assert a4.action_type == "set_variable"

    # Run command
    a5 = TriggerAction("run_command", {"command": "echo", "args": "hello,world"})
    assert a5.action_type == "run_command"

    # Play sound
    a6 = TriggerAction("play_sound", {"sound_id": "alert", "volume": "80"})
    assert a6.action_type == "play_sound"

    # Send text
    a7 = TriggerAction("send_text", {"text": "response\n", "delay_ms": "100"})
    assert a7.action_type == "send_text"

    # Stop propagation
    a8 = TriggerAction("stop")
    assert a8.action_type == "stop"

    # All should be usable in add_trigger
    term = Terminal(80, 24)
    trigger_id = term.add_trigger(
        "all_actions", "TRIGGER", [a1, a2, a3, a4, a5, a6, a7, a8]
    )
    assert trigger_id > 0


def test_trigger_highlights():
    """Test highlight overlays from trigger actions."""
    term = Terminal(80, 24)
    action = TriggerAction("highlight", {"bg_r": "255", "bg_g": "0", "bg_b": "0"})
    term.add_trigger("test", "HIGHLIGHT", [action])

    term.process_str("HIGHLIGHT this\n")
    term.process_trigger_scans()

    highlights = term.get_trigger_highlights()
    assert len(highlights) == 1
    row, col_start, col_end, fg, bg = highlights[0]
    assert row == 0
    assert bg == (255, 0, 0)
    assert fg is None

    # Clear
    term.clear_trigger_highlights()
    assert len(term.get_trigger_highlights()) == 0


def test_trigger_enable_disable():
    """Test enabling and disabling triggers."""
    term = Terminal(80, 24)
    trigger_id = term.add_trigger("test", "MATCH", [])

    assert term.set_trigger_enabled(trigger_id, False) is True
    t = term.get_trigger(trigger_id)
    assert t.enabled is False

    assert term.set_trigger_enabled(trigger_id, True) is True
    t = term.get_trigger(trigger_id)
    assert t.enabled is True


def test_trigger_invalid_regex():
    """Test that invalid regex raises ValueError."""
    term = Terminal(80, 24)
    with pytest.raises(ValueError):
        term.add_trigger("bad", "[invalid", [])


def test_trigger_invalid_action():
    """Test that invalid action type raises ValueError."""
    term = Terminal(80, 24)
    bad_action = TriggerAction("nonexistent_action")
    with pytest.raises(ValueError):
        term.add_trigger("test", "MATCH", [bad_action])


def test_trigger_repr():
    """Test repr methods."""
    action = TriggerAction("highlight", {"bg_r": "255"})
    assert "highlight" in repr(action)

    term = Terminal(80, 24)
    tid = term.add_trigger("test_repr", "PATTERN", [])
    t = term.get_trigger(tid)
    assert "test_repr" in repr(t)


def test_coprocess_config():
    """Test CoprocessConfig construction."""
    config = CoprocessConfig("cat")
    assert config.command == "cat"
    assert config.args == []
    assert config.cwd is None
    assert config.env == {}
    assert config.copy_terminal_output is True

    config2 = CoprocessConfig(
        "grep",
        args=["ERROR"],
        cwd="/tmp",
        env={"LC_ALL": "C"},
        copy_terminal_output=False,
    )
    assert config2.command == "grep"
    assert config2.args == ["ERROR"]
    assert config2.cwd == "/tmp"
    assert config2.copy_terminal_output is False
    assert "grep" in repr(config2)
