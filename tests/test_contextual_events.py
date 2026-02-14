"""Tests for contextual awareness API events (issue #37)."""

import par_term_emu_core_rust as pte


def test_zone_opened_event() -> None:
    """ZoneOpened event fires on OSC 133 A."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    events = term.poll_events()
    zone_opened = [e for e in events if e["type"] == "zone_opened"]
    assert len(zone_opened) >= 1
    assert zone_opened[0]["zone_type"] == "prompt"
    assert "zone_id" in zone_opened[0]
    assert "abs_row_start" in zone_opened[0]


def test_zone_closed_event() -> None:
    """ZoneClosed event fires when zone transitions."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.poll_events()  # drain
    term.process(b"\x1b]133;B\x1b\\")
    events = term.poll_events()
    zone_closed = [e for e in events if e["type"] == "zone_closed"]
    assert len(zone_closed) >= 1
    assert zone_closed[0]["zone_type"] == "prompt"


def test_zone_closed_with_exit_code() -> None:
    """ZoneClosed for output zone includes exit code."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.process(b"\x1b]133;B\x1b\\")
    term.process(b"\x1b]133;C\x1b\\")
    term.poll_events()
    term.process(b"\x1b]133;D;0\x1b\\")
    events = term.poll_events()
    zone_closed = [e for e in events if e["type"] == "zone_closed"]
    output_closes = [e for e in zone_closed if e["zone_type"] == "output"]
    assert len(output_closes) >= 1
    assert output_closes[0].get("exit_code") == "0"


def test_zone_ids_monotonic() -> None:
    """Zone IDs increase monotonically."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.process(b"\x1b]133;B\x1b\\")
    term.process(b"\x1b]133;C\x1b\\")
    events = term.poll_events()
    zone_ids = [int(e["zone_id"]) for e in events if e["type"] == "zone_opened"]
    assert zone_ids == sorted(zone_ids)
    assert len(zone_ids) == 3


def test_zone_opened_includes_all_types() -> None:
    """ZoneOpened events cover prompt, command, and output zone types."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    term.process(b"\x1b]133;B\x1b\\")
    term.process(b"\x1b]133;C\x1b\\")
    events = term.poll_events()
    zone_types = [e["zone_type"] for e in events if e["type"] == "zone_opened"]
    assert "prompt" in zone_types
    assert "command" in zone_types
    assert "output" in zone_types


def test_zone_closed_has_row_range() -> None:
    """ZoneClosed events include abs_row_start and abs_row_end."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\$ \r\n")
    term.process(b"\x1b]133;B\x1b\\")
    events = term.poll_events()
    zone_closed = [e for e in events if e["type"] == "zone_closed"]
    assert len(zone_closed) >= 1
    assert "abs_row_start" in zone_closed[0]
    assert "abs_row_end" in zone_closed[0]


def test_environment_changed_cwd() -> None:
    """EnvironmentChanged fires on CWD change."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file:///home/user/project\x1b\\")
    events = term.poll_events()
    env_events = [e for e in events if e["type"] == "environment_changed"]
    cwd_events = [e for e in env_events if e["key"] == "cwd"]
    assert len(cwd_events) >= 1
    assert cwd_events[0]["value"] == "/home/user/project"


def test_environment_changed_hostname() -> None:
    """EnvironmentChanged fires for hostname on OSC 7."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file://myhost/home/user\x1b\\")
    events = term.poll_events()
    env_events = [e for e in events if e["type"] == "environment_changed"]
    host_events = [e for e in env_events if e["key"] == "hostname"]
    assert len(host_events) >= 1
    assert host_events[0]["value"] == "myhost"


def test_environment_changed_old_value() -> None:
    """EnvironmentChanged includes old_value when cwd changes."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file:///home/user/dir1\x1b\\")
    term.poll_events()  # drain
    term.process(b"\x1b]7;file:///home/user/dir2\x1b\\")
    events = term.poll_events()
    cwd_events = [
        e for e in events if e["type"] == "environment_changed" and e["key"] == "cwd"
    ]
    assert len(cwd_events) >= 1
    assert cwd_events[0]["value"] == "/home/user/dir2"
    assert cwd_events[0].get("old_value") == "/home/user/dir1"


def test_remote_host_transition_osc1337() -> None:
    """RemoteHostTransition fires on OSC 1337 RemoteHost."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\")
    events = term.poll_events()
    host_events = [e for e in events if e["type"] == "remote_host_transition"]
    assert len(host_events) >= 1
    assert host_events[0]["hostname"] == "server1"
    assert host_events[0].get("username") == "alice"


def test_remote_host_transition_osc7() -> None:
    """RemoteHostTransition fires on OSC 7 hostname change."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file://remotehost/home/user\x1b\\")
    events = term.poll_events()
    host_events = [e for e in events if e["type"] == "remote_host_transition"]
    assert len(host_events) >= 1
    assert host_events[0]["hostname"] == "remotehost"


def test_remote_host_transition_includes_old_values() -> None:
    """RemoteHostTransition includes old_hostname and old_username."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\")
    term.poll_events()  # drain
    term.process(b"\x1b]1337;RemoteHost=bob@server2\x1b\\")
    events = term.poll_events()
    host_events = [e for e in events if e["type"] == "remote_host_transition"]
    assert len(host_events) >= 1
    assert host_events[0]["hostname"] == "server2"
    assert host_events[0].get("username") == "bob"
    assert host_events[0].get("old_hostname") == "server1"
    assert host_events[0].get("old_username") == "alice"


def test_event_subscription_zone_events() -> None:
    """New zone event types work with subscription filtering."""
    term = pte.Terminal(80, 24)
    term.set_event_subscription(["zone_opened", "zone_closed"])
    term.process(b"\x1b]133;A\x1b\\")
    events = term.poll_subscribed_events()
    assert all(e["type"] in ("zone_opened", "zone_closed") for e in events)
    assert len(events) >= 1


def test_event_subscription_environment() -> None:
    """EnvironmentChanged works with subscription filtering."""
    term = pte.Terminal(80, 24)
    term.set_event_subscription(["environment_changed"])
    term.process(b"\x1b]7;file:///home/user\x1b\\")
    events = term.poll_subscribed_events()
    assert all(e["type"] == "environment_changed" for e in events)
    assert len(events) >= 1


def test_event_subscription_remote_host() -> None:
    """RemoteHostTransition works with subscription filtering."""
    term = pte.Terminal(80, 24)
    term.set_event_subscription(["remote_host_transition"])
    term.process(b"\x1b]1337;RemoteHost=alice@server1\x1b\\")
    events = term.poll_subscribed_events()
    assert all(e["type"] == "remote_host_transition" for e in events)
    assert len(events) >= 1


def test_poll_events_drains_queue() -> None:
    """Calling poll_events drains the event queue."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]133;A\x1b\\")
    events1 = term.poll_events()
    assert len(events1) >= 1
    events2 = term.poll_events()
    assert len(events2) == 0


def test_zone_scrolled_out_event() -> None:
    """ZoneScrolledOut fires when zones are evicted from scrollback."""
    term = pte.Terminal(80, 5, scrollback=10)
    term.process(b"\x1b]133;A\x07")
    term.process(b"\x1b]133;B\x07")
    term.process(b"\x1b]133;C\x07")
    term.poll_events()  # drain
    # Generate enough output to overflow scrollback and evict zones
    for i in range(30):
        term.process(f"line {i}\r\n".encode())
    term.process(b"\x1b]133;D;0\x07")
    events = term.poll_events()
    scrolled_out = [e for e in events if e["type"] == "zone_scrolled_out"]
    assert len(scrolled_out) >= 1
    assert "zone_id" in scrolled_out[0]
    assert "zone_type" in scrolled_out[0]


def test_multiple_cwd_changes_emit_multiple_events() -> None:
    """Each CWD change emits its own EnvironmentChanged event."""
    term = pte.Terminal(80, 24)
    term.process(b"\x1b]7;file:///dir1\x1b\\")
    term.process(b"\x1b]7;file:///dir2\x1b\\")
    term.process(b"\x1b]7;file:///dir3\x1b\\")
    events = term.poll_events()
    cwd_events = [
        e for e in events if e["type"] == "environment_changed" and e["key"] == "cwd"
    ]
    assert len(cwd_events) == 3
    assert cwd_events[0]["value"] == "/dir1"
    assert cwd_events[1]["value"] == "/dir2"
    assert cwd_events[2]["value"] == "/dir3"
