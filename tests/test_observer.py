"""Tests for terminal observer API."""

from __future__ import annotations

import asyncio

from par_term_emu_core_rust import Terminal
from par_term_emu_core_rust.observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)


class TestNativeEventDicts:
    """QA-119: poll_events dicts carry native int/bool/None values.

    These assertions fail against the pre-0.51 stringly-typed output, which
    ``poll_events_legacy`` still returns for one release.
    """

    def test_numeric_and_bool_fields_are_native(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x1b\\")  # zone_opened (zone_id, abs_row_start)
        term.process(b"\x1b[4h")  # insert_mode on -> mode_changed(enabled=True)
        events = term.poll_events()
        zone = next(e for e in events if e["type"] == "zone_opened")
        assert isinstance(zone["zone_id"], int)
        assert isinstance(zone["abs_row_start"], int)
        mode = next(e for e in events if e["type"] == "mode_changed")
        assert mode["enabled"] is True

    def test_unset_optional_field_is_none(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # First SetUserVar has no previous value: old_value must be None.
        term.process(b"\x1b]1337;SetUserVar=qa119var=cXV1eA==\x07")  # "quux"
        events = term.poll_events()
        var = next(e for e in events if e["type"] == "user_var_changed")
        assert var["value"] == "quux"
        assert var["old_value"] is None

    def test_legacy_poll_returns_stringly_shape(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x1b\\")
        term.process(b"\x1b[4h")
        events = term.poll_events_legacy()
        zone = next(e for e in events if e["type"] == "zone_opened")
        assert isinstance(zone["zone_id"], str)
        mode = next(e for e in events if e["type"] == "mode_changed")
        assert mode["enabled"] == "true"

    def test_legacy_poll_omits_unset_optional_fields(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]1337;SetUserVar=qa119var=cXV1eA==\x07")
        events = term.poll_events_legacy()
        var = next(e for e in events if e["type"] == "user_var_changed")
        assert var["value"] == "quux"
        assert "old_value" not in var

    def test_poll_subscribed_events_native_and_legacy(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.set_event_subscription(["user_var_changed"])
        term.process(b"\x1b]1337;SetUserVar=qa119sub=cXV1eA==\x07")
        term.process(b"\x1b[4h")  # not subscribed: must be filtered out
        native = term.poll_subscribed_events()
        assert [e["type"] for e in native] == ["user_var_changed"]
        assert native[0]["old_value"] is None

        term.process(b"\x1b]1337;SetUserVar=qa119sub=YWdhaW4=\x07")  # "again"
        legacy = term.poll_subscribed_events_legacy()
        assert [e["type"] for e in legacy] == ["user_var_changed"]
        assert legacy[0]["old_value"] == "quux"
        assert isinstance(legacy[0]["old_value"], str)


class TestSyncObserver:
    def test_add_and_remove_observer(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = term.add_observer(lambda e: events.append(e))
        assert term.observer_count() == 1
        assert term.remove_observer(observer_id)
        assert term.observer_count() == 0

    def test_observer_receives_bell(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        assert any(e["type"] == "bell" for e in events)

    def test_observer_receives_title_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x1b]0;Test Title\x07")
        assert any(
            e["type"] == "title_changed" and e["title"] == "Test Title" for e in events
        )

    def test_observer_with_filter(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e), kinds=["title_changed"])
        term.process(b"\x07")
        assert not any(e["type"] == "bell" for e in events)
        term.process(b"\x1b]0;Filtered\x07")
        assert any(e["type"] == "title_changed" for e in events)

    def test_observer_kind_filter_screen_cleared(self) -> None:
        """kinds=["screen_cleared"] must subscribe, not silently drop.

        Regression test: parse_event_kind had no "screen_cleared" arm, so the
        filter_map dropped the string and the observer (subscribed with an
        empty kind set) never fired.
        """
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e), kinds=["screen_cleared"])
        term.process(b"\x07")
        assert not any(e["type"] == "bell" for e in events)
        term.process(b"\x1b[2J")
        assert any(
            e["type"] == "screen_cleared" and e["include_scrollback"] is False
            for e in events
        )

    def test_multiple_observers(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events1: list[dict[str, str]] = []
        events2: list[dict[str, str]] = []
        term.add_observer(lambda e: events1.append(e))
        term.add_observer(lambda e: events2.append(e))
        term.process(b"\x07")
        assert len(events1) > 0
        assert len(events2) > 0

    def test_observer_removal_stops_delivery(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        count_after_first = len(events)
        term.remove_observer(observer_id)
        term.process(b"\x07")
        assert len(events) == count_after_first

    def test_poll_events_still_works_with_observer(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        term.add_observer(lambda e: events.append(e))
        term.process(b"\x07")
        polled = term.poll_events()
        assert len(polled) > 0

    def test_remove_nonexistent_observer(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert not term.remove_observer(99999)

    def test_observer_count_with_multiple(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.observer_count() == 0
        id1 = term.add_observer(lambda _e: None)
        assert term.observer_count() == 1
        id2 = term.add_observer(lambda _e: None)
        assert term.observer_count() == 2
        term.remove_observer(id1)
        assert term.observer_count() == 1
        term.remove_observer(id2)
        assert term.observer_count() == 0


class TestAsyncObserver:
    def test_async_observer_returns_queue(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer()
        assert observer_id >= 0
        assert hasattr(queue, "get")
        assert hasattr(queue, "put_nowait")
        term.remove_observer(observer_id)

    def test_async_observer_receives_events(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer()
        term.process(b"\x1b]0;Async Test\x07")
        events = []
        while not queue.empty():
            events.append(queue.get_nowait())
        assert any(
            e["type"] == "title_changed" and e["title"] == "Async Test" for e in events
        )
        term.remove_observer(observer_id)

    def test_async_observer_with_filter(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        observer_id, queue = term.add_async_observer(kinds=["title_changed"])
        term.process(b"\x07")
        term.process(b"\x1b]0;Filtered Async\x07")
        events = []
        while not queue.empty():
            events.append(queue.get_nowait())
        assert not any(e["type"] == "bell" for e in events)
        assert any(e["type"] == "title_changed" for e in events)
        term.remove_observer(observer_id)

    def test_async_observer_with_asyncio_loop(self) -> None:
        async def run_test() -> list[dict[str, str]]:
            term = Terminal(80, 24, scrollback=100)
            observer_id, queue = term.add_async_observer()
            term.process(b"\x1b]0;Async Loop\x07")
            events = []
            while not queue.empty():
                event = queue.get_nowait()
                events.append(event)
            term.remove_observer(observer_id)
            return events

        events = asyncio.run(run_test())
        assert any(e["type"] == "title_changed" for e in events)


class TestConvenienceWrappers:
    def test_on_command_complete(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_command_complete(term, lambda e: events.append(e))
        # Simulate shell integration sequence: prompt_start, command_start,
        # command_executed, command_finished
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        assert len(events) > 0
        assert all(e.get("event_type") == "command_finished" for e in events)
        term.remove_observer(observer_id)

    def test_on_zone_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_zone_change(term, lambda e: events.append(e))
        # Shell integration sequences create zones
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        assert any(e["type"] == "zone_opened" for e in events)
        term.remove_observer(observer_id)

    def test_on_cwd_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_cwd_change(term, lambda e: events.append(e))
        term.process(b"\x1b]7;file:///home/user/test\x07")
        assert any(e["type"] == "cwd_changed" for e in events)
        term.remove_observer(observer_id)

    def test_on_title_change(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_title_change(term, lambda e: events.append(e))
        term.process(b"\x1b]0;New Title\x07")
        assert any(
            e["type"] == "title_changed" and e["title"] == "New Title" for e in events
        )
        term.remove_observer(observer_id)

    def test_on_bell(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        events: list[dict[str, str]] = []
        observer_id = on_bell(term, lambda e: events.append(e))
        term.process(b"\x07")
        assert any(e["type"] == "bell" for e in events)
        term.remove_observer(observer_id)
