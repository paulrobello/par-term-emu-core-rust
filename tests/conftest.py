"""Shared helpers for the Python test suite (QA-102).

``wait_for`` replaces fixed-duration sleeps as the synchronization pattern:
a fixed ``time.sleep(n)`` either wastes time when the machine is faster than
the guess or flakes when it is slower. Polling is bounded by the actual
condition instead of a guess about it.

Tests import it directly: ``from conftest import wait_for``. When ENH-011's
``wait_for_text`` API lands, the ``wait_for_text``-shaped call sites here
become thin wrappers over it.
"""

import time
from collections.abc import Callable


def wait_for[T](
    predicate: Callable[[], T],
    timeout: float = 5.0,
    interval: float = 0.02,
    description: str | None = None,
) -> T:
    """Poll ``predicate()`` until it returns a truthy value.

    Args:
        predicate: Condition to poll. May have side effects (e.g. ticking a
            macro between checks); it is called every ``interval`` seconds.
        timeout: Total seconds to keep polling before failing.
        interval: Seconds between polls.
        description: What the condition means, for the failure message.

    Returns:
        The first truthy result of ``predicate()``.

    Raises:
        AssertionError: If the predicate never holds within ``timeout``.
    """
    deadline = time.monotonic() + timeout
    while True:
        result = predicate()
        if result:
            return result
        if time.monotonic() >= deadline:
            what = description or getattr(predicate, "__name__", repr(predicate))
            raise AssertionError(f"condition not met within {timeout}s: {what}")
        time.sleep(interval)
