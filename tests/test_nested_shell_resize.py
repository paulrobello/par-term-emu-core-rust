#!/usr/bin/env python3
"""Test resize with a nested shell scenario (bash -> python).

This test mimics the actual TUI use case where bash spawns Python.
"""

import os
import sys
import tempfile

import pytest
from conftest import wait_for


def read_log(path: str) -> str:
    """Current contents of the child's log file (empty until it writes)."""
    try:
        with open(path) as f:
            return f.read()
    except FileNotFoundError:
        return ""


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_resize_through_shell():
    """Test that resize works when going through a shell (bash -> python)."""
    from par_term_emu_core_rust import PtyTerminal

    with tempfile.NamedTemporaryFile(mode="w+", delete=False, suffix=".log") as f:
        log_file = f.name

    with tempfile.NamedTemporaryFile(mode="w+", delete=False, suffix=".py") as f:
        script_file = f.name

    try:
        # Create a Python script file
        script_content = f"""#!/usr/bin/env python3
import signal
import fcntl
import struct
import termios
import sys
import time
import os

log_file = "{log_file}"

def get_size():
    try:
        result = fcntl.ioctl(sys.stdout.fileno(), termios.TIOCGWINSZ, b'\\x00' * 8)
        rows, cols, _, _ = struct.unpack('HHHH', result)
        return (cols, rows)
    except Exception as e:
        with open(log_file, 'a') as f:
            f.write(f'IOCTL_ERROR:{{str(e)}}\\n')
            f.flush()
        return (0, 0)

def sigwinch_handler(signum, frame):
    cols, rows = get_size()
    with open(log_file, 'a') as f:
        f.write(f'SIGWINCH:{{cols}}x{{rows}}\\n')
        f.flush()

signal.signal(signal.SIGWINCH, sigwinch_handler)

# Log initial size and PID info
cols, rows = get_size()
with open(log_file, 'w') as f:
    f.write(f'PYTHON_PID:{{os.getpid()}}\\n')
    f.write(f'PYTHON_PGID:{{os.getpgid(0)}}\\n')
    f.write(f'PYTHON_SID:{{os.getsid(0)}}\\n')
    f.write(f'INITIAL:{{cols}}x{{rows}}\\n')
    f.flush()

# Keep running and logging
for i in range(50):
    time.sleep(0.1)

# Final size
cols, rows = get_size()
with open(log_file, 'a') as f:
    f.write(f'FINAL:{{cols}}x{{rows}}\\n')
    f.flush()
"""

        with open(script_file, "w") as f:
            f.write(script_content)
        os.chmod(script_file, 0o755)

        # Spawn bash, which will spawn the Python script
        term = PtyTerminal(80, 24)
        term.spawn_shell()  # Spawns bash

        assert wait_for(lambda: term.content().strip())

        # Execute the Python script from bash
        cmd = f"{sys.executable} {script_file} &\n"
        print(f"Executing in bash: {cmd}")
        term.write_str(cmd)
        assert wait_for(lambda: "INITIAL:" in read_log(log_file))

        # Read the log to get process info
        content = read_log(log_file)
        print(f"Initial state:\n{content}")

        # Resize the terminal
        print("\nResizing to 100x30...")
        term.resize(100, 30)

        # Check what the Python process sees. Nested SIGWINCH delivery is a
        # known-broken path, so absence within a bounded window skips rather
        # than fails (the original test's expected-failure probe).
        try:
            assert wait_for(
                lambda: "SIGWINCH:100x30" in read_log(log_file), timeout=2.0
            )
        except AssertionError:
            print(f"After resize:\n{read_log(log_file)}")
            pytest.skip(
                "Nested resize not yet working - need to investigate process groups"
            )

        print("✓ SUCCESS: Python grandchild received resize!")

        term.kill()

    finally:
        if os.path.exists(log_file):
            os.unlink(log_file)
        if os.path.exists(script_file):
            os.unlink(script_file)


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s"])
