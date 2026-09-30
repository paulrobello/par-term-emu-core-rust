"""The Python debug log must not be a symlink or disclosure target (SEC-131).

The log lives in the shared, world-writable temp directory, so it matches
``src/debug.rs``: a PID-suffixed name, created owner-only, and never opened
through a symlink planted at the path (logging is disabled instead).
"""

import importlib
import os
import stat
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest
from par_term_emu_core_rust import debug

pytestmark = pytest.mark.skipif(
    sys.platform == "win32", reason="file modes and O_NOFOLLOW are POSIX-only"
)


@pytest.fixture
def log_path(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Iterator[Path]:
    """Point a fresh (non-singleton-cached) logger at a per-test file."""
    path = tmp_path / "log.txt"
    monkeypatch.setenv("DEBUG_LEVEL", "1")
    monkeypatch.setattr(debug, "DEBUG_FILE", path)
    monkeypatch.setattr(debug.DebugLogger, "_instance", None)
    yield path
    logger = debug.DebugLogger._instance
    if logger is not None and logger.file_handle is not None:
        logger.file_handle.close()
        logger.file_handle = None


def test_debug_log_is_owner_only(log_path: Path) -> None:
    logger = debug.DebugLogger()
    assert logger.file_handle is not None
    assert stat.S_IMODE(os.stat(log_path).st_mode) == 0o600


def test_debug_log_refuses_a_planted_symlink(tmp_path: Path, log_path: Path) -> None:
    victim = tmp_path / "victim"
    victim.write_text("keep")
    os.symlink(victim, log_path)

    logger = debug.DebugLogger()

    assert victim.read_text() == "keep"
    assert logger.level == debug.DebugLevel.OFF
    assert logger.file_handle is None


def test_debug_log_name_carries_the_pid(monkeypatch: pytest.MonkeyPatch) -> None:
    # Reloading re-runs the module-level logger; keep it from opening a real
    # file in the shared temp dir.
    monkeypatch.delenv("DEBUG_LEVEL", raising=False)
    reloaded = importlib.reload(debug)
    assert str(os.getpid()) in reloaded.DEBUG_FILE.name
