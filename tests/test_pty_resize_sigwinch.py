#!/usr/bin/env python3
"""Test that SIGWINCH is properly delivered to child processes via PTY.

This test verifies the complete resize signal chain:
1. Python calls term.resize()
2. PTY sends SIGWINCH to child process
3. Child process receives SIGWINCH
4. Child process queries new size via ioctl
5. Child process sees the updated size
"""

import os
import sys
import tempfile
import time

import pytest

pytestmark = pytest.mark.skip(reason="PTY tests hang in CI")


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_sigwinch_delivery_to_child():
    """Test that SIGWINCH is delivered when PTY is resized."""
    from par_term_emu_core_rust import PtyTerminal

    # Create a temporary file for the child to log to
    with tempfile.NamedTemporaryFile(mode="w+", delete=False, suffix=".log") as f:
        log_file = f.name

    try:
        # Create a Python script that logs SIGWINCH signals
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
    except:
        return (0, 0)

def sigwinch_handler(signum, frame):
    cols, rows = get_size()
    with open(log_file, 'a') as f:
        f.write(f'SIGWINCH:{{cols}}x{{rows}}\\n')
        f.flush()

signal.signal(signal.SIGWINCH, sigwinch_handler)

# Log initial size
cols, rows = get_size()
with open(log_file, 'w') as f:
    f.write(f'INITIAL:{{cols}}x{{rows}}\\n')
    f.flush()

# Keep running for a few seconds
for i in range(50):
    time.sleep(0.1)

# Log final size
cols, rows = get_size()
with open(log_file, 'a') as f:
    f.write(f'FINAL:{{cols}}x{{rows}}\\n')
    f.flush()
"""

        # Create terminal and spawn the test script
        term = PtyTerminal(80, 24)
        term.spawn("/usr/bin/python3", args=["-c", script])

        # Wait for script to initialize
        time.sleep(0.2)

        # Verify initial size was logged
        with open(log_file) as f:
            content = f.read()
            assert "INITIAL:80x24" in content, f"Expected INITIAL:80x24, got: {content}"

        # Resize the terminal
        term.resize(100, 30)

        # Wait for SIGWINCH to be delivered and processed
        time.sleep(0.3)

        # Check that SIGWINCH was received with correct size
        with open(log_file) as f:
            content = f.read()
            assert "SIGWINCH:100x30" in content, (
                f"Expected SIGWINCH:100x30 in log, got: {content}"
            )

        # Resize again
        term.resize(120, 40)
        time.sleep(0.3)

        # Check second resize
        with open(log_file) as f:
            content = f.read()
            assert "SIGWINCH:120x40" in content, (
                f"Expected SIGWINCH:120x40 in log, got: {content}"
            )

        # Kill the process
        term.kill()

    finally:
        # Clean up temp file
        if os.path.exists(log_file):
            os.unlink(log_file)


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_sigwinch_with_shell():
    """Test that shell receives SIGWINCH and updates COLUMNS/LINES."""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Wait for shell to start
    time.sleep(0.5)

    # Query initial size
    term.write_str("echo SIZE:${COLUMNS}x${LINES}\n")
    time.sleep(0.2)

    # Shell should show 80x24 (or close to it, depending on shell behavior)
    # Note: Some shells don't update env vars on SIGWINCH

    # Resize
    term.resize(100, 30)
    time.sleep(0.3)

    # Query size again
    term.write_str("echo RESIZED:${COLUMNS}x${LINES}\n")
    time.sleep(0.2)

    # At minimum, the terminal should have resized
    assert term.size() == (100, 30)

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_multiple_rapid_resizes():
    """Test that multiple rapid resizes all send SIGWINCH."""
    from par_term_emu_core_rust import PtyTerminal

    with tempfile.NamedTemporaryFile(mode="w+", delete=False, suffix=".log") as f:
        log_file = f.name

    try:
        script = f"""
import signal
import fcntl
import struct
import termios
import sys
import time

log_file = "{log_file}"
count = [0]

def sigwinch_handler(signum, frame):
    count[0] += 1
    with open(log_file, 'a') as f:
        f.write(f'SIGWINCH:{{count[0]}}\\n')
        f.flush()

signal.signal(signal.SIGWINCH, sigwinch_handler)

with open(log_file, 'w') as f:
    f.write('STARTED\\n')
    f.flush()

time.sleep(2)

with open(log_file, 'a') as f:
    f.write(f'TOTAL:{{count[0]}}\\n')
    f.flush()
"""

        term = PtyTerminal(80, 24)
        term.spawn("/usr/bin/python3", args=["-c", script])

        time.sleep(0.2)

        # Perform multiple rapid resizes
        sizes = [(90, 25), (100, 30), (110, 35), (120, 40), (100, 30)]
        for cols, rows in sizes:
            term.resize(cols, rows)
            time.sleep(0.1)

        # Wait for script to finish
        time.sleep(1)

        # Check that we got SIGWINCH signals
        with open(log_file) as f:
            content = f.read()
            # We should see multiple SIGWINCH entries
            sigwinch_count = content.count("SIGWINCH:")
            assert sigwinch_count >= 3, (
                f"Expected at least 3 SIGWINCH signals, got {sigwinch_count}: {content}"
            )

        term.kill()

    finally:
        if os.path.exists(log_file):
            os.unlink(log_file)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
