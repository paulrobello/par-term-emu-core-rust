"""Screenshot tests that spawn a real PTY.

Part of the PTY test family (QA-111): these run in every local ``make test`` /
``make test-pty``, while CI keeps its ``--ignore`` entries for the family
(GitHub runners hang on PTY I/O). They were skipped unconditionally from
2025-11 until QA-130 gave the read-only PtySession getters the read lock, so
a screenshot render can no longer stall behind the reader thread's ``process()``.
"""

import os
import tempfile

import pytest
from conftest import wait_for
from par_term_emu_core_rust import PtyTerminal


# PTY spawns and screenshot renders stall past the 5 s global budget under
# machine load — hook runs on 2026-09-30 timed out inside the bare spawn.
@pytest.mark.timeout(30)
class TestPtyTerminalScreenshot:
    """Test screenshot functionality with PtyTerminal"""

    def test_pty_screenshot(self):
        """Test screenshot from PTY terminal"""
        with PtyTerminal(80, 24) as pty:
            # Spawn a shell to activate the PTY
            pty.spawn_shell()
            # Wait for the shell prompt to render before screenshotting
            assert wait_for(lambda: pty.content().strip())

            # Take screenshot (should capture shell prompt)
            png_bytes = pty.screenshot()
            assert len(png_bytes) > 0
            assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"

            # Clean exit
            pty.write_str("exit\n")

    def test_pty_screenshot_to_file(self):
        """Test saving PTY screenshot to file"""
        with PtyTerminal(80, 24) as pty:
            pty.spawn_shell()
            assert wait_for(lambda: pty.content().strip())

            with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
                filename = f.name

            try:
                pty.screenshot_to_file(filename)
                assert os.path.exists(filename)
                assert os.path.getsize(filename) > 0
            finally:
                if os.path.exists(filename):
                    os.remove(filename)

            pty.write_str("exit\n")
