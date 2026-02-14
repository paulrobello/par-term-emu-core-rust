"""Convenience wrappers for terminal observer patterns.

These functions simplify common observer use cases by wrapping
:meth:`Terminal.add_observer` with appropriate event filters.
"""

from __future__ import annotations

from collections.abc import Callable
from typing import Any


def on_command_complete(
    terminal: Any, callback: Callable[[dict[str, str]], None]
) -> int:
    """Register callback for command completion events.

    Fires when a shell integration ``command_finished`` event is received
    (OSC 133;D).

    Args:
        terminal: A :class:`Terminal` instance.
        callback: Called with the event dict when a command finishes.

    Returns:
        Observer ID for later removal via ``terminal.remove_observer()``.
    """

    def handler(event: dict[str, str]) -> None:
        if (
            event.get("type") == "shell_integration"
            and event.get("event_type") == "command_finished"
        ):
            callback(event)

    return terminal.add_observer(handler, kinds=["shell_integration"])


def on_zone_change(terminal: Any, callback: Callable[[dict[str, str]], None]) -> int:
    """Register callback for zone lifecycle events.

    Fires on zone open, close, and scroll-out events.

    Args:
        terminal: A :class:`Terminal` instance.
        callback: Called with the event dict for each zone event.

    Returns:
        Observer ID for later removal via ``terminal.remove_observer()``.
    """
    return terminal.add_observer(
        callback, kinds=["zone_opened", "zone_closed", "zone_scrolled_out"]
    )


def on_cwd_change(terminal: Any, callback: Callable[[dict[str, str]], None]) -> int:
    """Register callback for working directory changes.

    Fires when OSC 7 updates the current working directory.

    Args:
        terminal: A :class:`Terminal` instance.
        callback: Called with the event dict containing ``new_cwd``.

    Returns:
        Observer ID for later removal via ``terminal.remove_observer()``.
    """
    return terminal.add_observer(callback, kinds=["cwd_changed"])


def on_title_change(terminal: Any, callback: Callable[[dict[str, str]], None]) -> int:
    """Register callback for terminal title changes.

    Fires when OSC 0/2 updates the terminal title.

    Args:
        terminal: A :class:`Terminal` instance.
        callback: Called with the event dict containing ``title``.

    Returns:
        Observer ID for later removal via ``terminal.remove_observer()``.
    """
    return terminal.add_observer(callback, kinds=["title_changed"])


def on_bell(terminal: Any, callback: Callable[[dict[str, str]], None]) -> int:
    """Register callback for bell events.

    Fires when BEL (0x07) or other bell sequences are processed.

    Args:
        terminal: A :class:`Terminal` instance.
        callback: Called with the event dict for each bell event.

    Returns:
        Observer ID for later removal via ``terminal.remove_observer()``.
    """
    return terminal.add_observer(callback, kinds=["bell"])
