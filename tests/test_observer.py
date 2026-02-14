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
        id1 = term.add_observer(lambda e: None)
        assert term.observer_count() == 1
        id2 = term.add_observer(lambda e: None)
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
