#!/usr/bin/env python3
"""Test to verify that ioctl(TIOCGWINSZ) returns the updated size after resize.

This test checks the actual ioctl behavior, not just SIGWINCH delivery.
"""

import os
import sys
import tempfile

import pytest
from conftest import wait_for


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_ioctl_returns_updated_size():
    """Test that ioctl(TIOCGWINSZ) returns the new size after PTY resize."""
    from par_term_emu_core_rust import PtyTerminal

    with tempfile.NamedTemporaryFile(mode="w+", delete=False, suffix=".log") as f:
        log_file = f.name

    try:
        # Script that continuously logs the size via ioctl
        script = f"""
import signal
import fcntl
import struct
import termios
import sys
import time

log_file = "{log_file}"

def get_size():
    try:
        result = fcntl.ioctl(sys.stdout.fileno(), termios.TIOCGWINSZ, b'\\x00' * 8)
        rows, cols, _, _ = struct.unpack('HHHH', result)
        return (cols, rows)
    except Exception as e:
        return (0, 0)

def sigwinch_handler(signum, frame):
    cols, rows = get_size()
    with open(log_file, 'a') as f:
        f.write(f'SIGWINCH_SIZE:{{cols}}x{{rows}}\\n')
        f.flush()

signal.signal(signal.SIGWINCH, sigwinch_handler)

# Log initial size
cols, rows = get_size()
with open(log_file, 'w') as f:
    f.write(f'INITIAL_SIZE:{{cols}}x{{rows}}\\n')
    f.flush()

# Poll size every 100ms and log any changes
last_size = (cols, rows)
for i in range(50):  # 5 seconds total
    time.sleep(0.1)
    cols, rows = get_size()
    if (cols, rows) != last_size:
        with open(log_file, 'a') as f:
            f.write(f'POLL_SIZE:{{cols}}x{{rows}}\\n')
            f.flush()
        last_size = (cols, rows)

# Log final size
cols, rows = get_size()
with open(log_file, 'a') as f:
    f.write(f'FINAL_SIZE:{{cols}}x{{rows}}\\n')
    f.flush()
"""

        def read_log() -> str:
            with open(log_file) as f:
                return f.read()

        term = PtyTerminal(80, 24)
        term.spawn("/usr/bin/python3", args=["-c", script])

        # Verify initial size (poll — subprocess startup is not instant)
        assert wait_for(lambda: "INITIAL_SIZE:80x24" in read_log()), (
            f"Expected INITIAL_SIZE:80x24, got: {read_log()}"
        )

        # Resize
        print("Resizing to 100x30...")
        term.resize(100, 30)
        assert wait_for(
            lambda: (
                "SIGWINCH_SIZE:100x30" in read_log() or "POLL_SIZE:100x30" in read_log()
            )
        ), f"Expected size 100x30 to be visible via ioctl, but got:\n{read_log()}"

        # Resize again
        print("Resizing to 120x40...")
        term.resize(120, 40)
        assert wait_for(
            lambda: (
                "SIGWINCH_SIZE:120x40" in read_log() or "POLL_SIZE:120x40" in read_log()
            )
        ), f"Expected size 120x40 to be visible via ioctl, but got:\n{read_log()}"

        term.kill()

    finally:
        if os.path.exists(log_file):
            os.unlink(log_file)


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s"])
