#!/usr/bin/env python3
"""Test to verify that ioctl(TIOCGWINSZ) returns the updated size after resize.

This test checks the actual ioctl behavior, not just SIGWINCH delivery.
"""

import os
import sys
import tempfile
import time

import pytest


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

        def wait_for(marker: str, timeout_s: float = 5.0) -> str:
            """Poll the log file for ``marker`` up to ``timeout_s`` seconds.

            Fixed ``time.sleep`` races the scheduler under CPU contention on
            loaded machines — the subprocess that writes the log file may
            not have run yet. Poll instead so we stay responsive without
            gambling on arbitrary sleep durations.
            """
            deadline = time.monotonic() + timeout_s
            content = ""
            while time.monotonic() < deadline:
                with open(log_file) as f:
                    content = f.read()
                if marker in content:
                    return content
                time.sleep(0.05)
            return content

        term = PtyTerminal(80, 24)
        term.spawn("/usr/bin/python3", args=["-c", script])

        # Verify initial size (poll — subprocess startup is not instant)
        content = wait_for("INITIAL_SIZE:80x24")
        print(f"After spawn: {content}")
        assert "INITIAL_SIZE:80x24" in content, (
            f"Expected INITIAL_SIZE:80x24, got: {content}"
        )

        def wait_for_either(markers: tuple[str, ...], timeout_s: float = 5.0) -> str:
            deadline = time.monotonic() + timeout_s
            content = ""
            while time.monotonic() < deadline:
                with open(log_file) as f:
                    content = f.read()
                if any(m in content for m in markers):
                    return content
                time.sleep(0.05)
            return content

        # Resize
        print("Resizing to 100x30...")
        term.resize(100, 30)
        content = wait_for_either(("SIGWINCH_SIZE:100x30", "POLL_SIZE:100x30"))
        print(f"After first resize: {content}")
        assert "SIGWINCH_SIZE:100x30" in content or "POLL_SIZE:100x30" in content, (
            f"Expected size 100x30 to be visible via ioctl, but got:\n{content}"
        )

        # Resize again
        print("Resizing to 120x40...")
        term.resize(120, 40)
        content = wait_for_either(("SIGWINCH_SIZE:120x40", "POLL_SIZE:120x40"))
        print(f"After second resize: {content}")
        assert "SIGWINCH_SIZE:120x40" in content or "POLL_SIZE:120x40" in content, (
            f"Expected size 120x40 to be visible via ioctl, but got:\n{content}"
        )

        term.kill()

    finally:
        if os.path.exists(log_file):
            os.unlink(log_file)


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s"])
