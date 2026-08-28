#!/usr/bin/env python3
"""Tests for OSC 1337 RemoteHost support."""

from par_term_emu_core_rust import Terminal


def _remote_host_seq(payload: str, bell: bool = False) -> str:
    """Build an OSC 1337 RemoteHost escape sequence."""
    term = "\x07" if bell else "\x1b\\"
    return f"\x1b]1337;RemoteHost={payload}{term}"


def test_remote_host_user_and_hostname():
    """Test parsing user@hostname format updates session variables."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq("alice@server1.example.com"))

    vars = term.get_badge_session_variables()
    assert vars.get("hostname") == "server1.example.com"
    assert vars.get("username") == "alice"


def test_remote_host_hostname_only():
    """Test parsing hostname without username."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq("myserver"))

    vars = term.get_badge_session_variables()
    assert vars.get("hostname") == "myserver"
    assert vars.get("username") is None


def test_remote_host_localhost_clears():
    """Test that localhost is treated as no remote host."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq("alice@remote"))
    assert term.get_badge_session_variables().get("hostname") == "remote"

    term.process_str(_remote_host_seq("alice@localhost"))
    assert term.get_badge_session_variables().get("hostname") is None
    assert term.get_badge_session_variables().get("username") == "alice"


def test_remote_host_bell_terminated():
    """Test BEL-terminated variant."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq("bob@host2", bell=True))

    vars = term.get_badge_session_variables()
    assert vars.get("hostname") == "host2"
    assert vars.get("username") == "bob"


def test_remote_host_emits_cwd_changed_event():
    """Test that CwdChanged event is emitted with hostname/username."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq("alice@remote-server"))

    events = term.poll_events()
    cwd_events = [e for e in events if e.get("type") == "cwd_changed"]
    assert len(cwd_events) == 1
    assert cwd_events[0]["hostname"] == "remote-server"
    assert cwd_events[0]["username"] == "alice"


def test_remote_host_preserves_cwd():
    """Test that RemoteHost does not clear existing cwd."""
    term = Terminal(80, 24)
    term.process_str("\x1b]7;file:///home/user/project\x1b\\")
    assert term.shell_integration_state().cwd == "/home/user/project"

    term.process_str(_remote_host_seq("alice@remote"))
    assert term.shell_integration_state().cwd == "/home/user/project"
    assert term.get_badge_session_variables().get("hostname") == "remote"


def test_remote_host_empty_payload_ignored():
    """Test that empty payload is silently ignored."""
    term = Terminal(80, 24)
    term.process_str(_remote_host_seq(""))
    assert term.get_badge_session_variables().get("hostname") is None
    assert term.get_badge_session_variables().get("username") is None


def test_remote_host_overrides_osc7():
    """Test that RemoteHost overrides previously set OSC 7 hostname."""
    term = Terminal(80, 24)
    term.process_str("\x1b]7;file://server1/home/user\x1b\\")
    assert term.get_badge_session_variables().get("hostname") == "server1"

    term.process_str(_remote_host_seq("bob@server2"))
    assert term.get_badge_session_variables().get("hostname") == "server2"
    assert term.get_badge_session_variables().get("username") == "bob"


def test_remote_host_event_subscription():
    """Test RemoteHost events work with event subscriptions."""
    term = Terminal(80, 24)
    term.set_event_subscription(["cwd_changed"])

    term.process_str(_remote_host_seq("alice@remote"))

    events = term.poll_subscribed_events()
    assert len(events) == 1
    assert events[0]["type"] == "cwd_changed"
    assert events[0]["hostname"] == "remote"
