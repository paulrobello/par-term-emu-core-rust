"""Shared harness for the `scripts/check_*.py` gates (QA-708).

The gates that report on stdout and exit 1 share three pieces: the repo
root, `fail()`, and the self-test step that runs a check against an
injected drift and requires it to fail naming the drift. Each gate keeps
its own check logic and CLI. Import it the way a gate script runs
(`python3 scripts/<gate>.py` puts `scripts/` on `sys.path`):

    from _gate_common import ROOT, expect_failure, fail
"""

from __future__ import annotations

import contextlib
import io
from collections.abc import Callable
from pathlib import Path
from typing import NoReturn

ROOT = Path(__file__).resolve().parent.parent


def fail(msg: str) -> NoReturn:
    """Print `msg` on stdout and exit 1: the gate's failure report."""
    print(msg)
    raise SystemExit(1)


def capture(run: Callable[[], object]) -> tuple[bool, str]:
    """Run `run` with stdout captured; return (whether it exited, the output)."""
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            run()
    except SystemExit:
        return True, buf.getvalue()
    return False, buf.getvalue()


def expect_failure(run: Callable[[], object], needle: str) -> None:
    """Self-test step: `run` must fail, and its report must contain `needle`."""
    failed, output = capture(run)
    if not failed:
        fail(f"self-test: expected a failure mentioning {needle!r}, got a pass")
    if needle not in output:
        fail(f"self-test: failure output lacks {needle!r}: {output}")
