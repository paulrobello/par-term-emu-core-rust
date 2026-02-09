#!/usr/bin/env python3
"""Tests for OSC 1337 SetUserVar support."""

import base64

from par_term_emu_core_rust import Terminal


def _set_user_var_seq(name: str, value: str) -> str:
    """Build an OSC 1337 SetUserVar escape sequence."""
    encoded = base64.b64encode(value.encode()).decode()
    return f"\x1b]1337;SetUserVar={name}={encoded}\x07"


def test_set_user_var_basic():
    """Test basic SetUserVar parsing and retrieval."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("hostname", "myhost.example.com"))

    assert term.get_user_var("hostname") == "myhost.example.com"


def test_get_user_var_not_set():
    """Test getting a variable that hasn't been set returns None."""
    term = Terminal(80, 24)
    assert term.get_user_var("nonexistent") is None


def test_get_user_vars_empty():
    """Test get_user_vars on a fresh terminal returns empty dict."""
    term = Terminal(80, 24)
    assert term.get_user_vars() == {}


def test_get_user_vars_multiple():
    """Test setting and retrieving multiple user variables."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("hostname", "server1"))
    term.process_str(_set_user_var_seq("username", "alice"))
    term.process_str(_set_user_var_seq("currentDir", "/home/alice"))

    vars = term.get_user_vars()
    assert vars["hostname"] == "server1"
    assert vars["username"] == "alice"
    assert vars["currentDir"] == "/home/alice"
    assert len(vars) == 3


def test_set_user_var_overwrite():
    """Test overwriting an existing user variable."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("host", "old"))
    assert term.get_user_var("host") == "old"

    term.process_str(_set_user_var_seq("host", "new"))
    assert term.get_user_var("host") == "new"


def test_user_var_changed_event():
    """Test that UserVarChanged events are emitted."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("myvar", "myval"))

    events = term.poll_events()
    user_var_events = [e for e in events if e.get("type") == "user_var_changed"]
    assert len(user_var_events) == 1
    assert user_var_events[0]["name"] == "myvar"
    assert user_var_events[0]["value"] == "myval"
    assert "old_value" not in user_var_events[0]


def test_user_var_changed_event_with_old_value():
    """Test that UserVarChanged events include old_value when updating."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("key", "first"))
    term.poll_events()  # drain

    term.process_str(_set_user_var_seq("key", "second"))
    events = term.poll_events()
    user_var_events = [e for e in events if e.get("type") == "user_var_changed"]
    assert len(user_var_events) == 1
    assert user_var_events[0]["name"] == "key"
    assert user_var_events[0]["value"] == "second"
    assert user_var_events[0]["old_value"] == "first"


def test_user_var_no_event_when_same_value():
    """Test that no event is emitted when setting the same value."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("key", "same"))
    term.poll_events()  # drain

    term.process_str(_set_user_var_seq("key", "same"))
    events = term.poll_events()
    user_var_events = [e for e in events if e.get("type") == "user_var_changed"]
    assert len(user_var_events) == 0


def test_user_var_event_subscription():
    """Test that user_var_changed events work with event subscriptions."""
    term = Terminal(80, 24)
    term.set_event_subscription(["user_var_changed"])

    term.process_str(_set_user_var_seq("host", "server1"))

    events = term.poll_subscribed_events()
    assert len(events) == 1
    assert events[0]["type"] == "user_var_changed"
    assert events[0]["name"] == "host"


def test_user_var_available_in_badge_session_variables():
    """Test that user vars are accessible via badge session variable API."""
    term = Terminal(80, 24)
    term.process_str(_set_user_var_seq("myvar", "testval"))

    # Should be accessible via get_badge_session_variable
    assert term.get_badge_session_variable("myvar") == "testval"

    # Should appear in get_badge_session_variables dict
    all_vars = term.get_badge_session_variables()
    assert "myvar" in all_vars
    assert all_vars["myvar"] == "testval"
