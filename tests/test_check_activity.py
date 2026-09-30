#!/usr/bin/env python3
"""check_activity behavior (QA-220).

check_activity was an empty stub in the Rust core; it now queues an
Activity/Visual event when update_activity() was called since the previous
event, rate-limited by activity_threshold seconds.
"""

import time

from par_term_emu_core_rust import Terminal


def make_term(activity_enabled: bool = True, threshold: int = 0) -> Terminal:
    term = Terminal(80, 24)
    # Timestamps have millisecond resolution: keep the construction and
    # update_activity() instants in distinct milliseconds so "activity since
    # construction" is well-defined.
    time.sleep(0.01)
    config = term.get_notification_config()
    config.activity_enabled = activity_enabled
    config.activity_threshold = threshold
    term.set_notification_config(config)
    return term


def test_no_event_without_activity():
    term = make_term()
    term.check_activity()
    assert len(term.get_notification_events()) == 0


def test_activity_event_fires_after_update_activity():
    term = make_term()
    term.update_activity()
    term.check_activity()
    events = term.get_notification_events()
    assert [(e.trigger, e.alert, e.message) for e in events] == [
        ("Activity", "Visual", "Terminal activity detected")
    ]


def test_disabled_config_never_fires():
    term = make_term(activity_enabled=False)
    term.update_activity()
    term.check_activity()
    assert len(term.get_notification_events()) == 0


def test_threshold_rate_limits_within_window():
    term = make_term(threshold=3600)
    term.update_activity()
    term.check_activity()
    assert len(term.get_notification_events()) == 0
