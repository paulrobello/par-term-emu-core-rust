#!/usr/bin/env python3
"""
Basic PTY Terminal Example

This example demonstrates basic PTY usage:
- Creating a PTY terminal
- Spawning a simple command
- Capturing output
- Getting exit status
"""

import sys
import tempfile
import time
from par_term_emu_core_rust import PtyTerminal


def get_echo_command():
    """Get platform-appropriate echo command."""
    if sys.platform == "win32":
        return ("cmd.exe", ["/C", "echo"])
    return ("/bin/echo", [])


def get_ls_command(path):
    """Get platform-appropriate directory listing command."""
    if sys.platform == "win32":
        return ("cmd.exe", ["/C", "dir", path])
    return ("/bin/ls", ["-la", path])


def main():
    print("=== Basic PTY Terminal Example ===\n")

    # Create a terminal
    term = PtyTerminal(80, 24)
    print(f"Created: {term}")
    print(f"Size: {term.size()}")
    print(f"Default shell: {PtyTerminal.get_default_shell()}\n")

    # Example 1: Run a simple command
    print("Example 1: Running 'echo Hello, PTY!'")
    print("-" * 50)

    cmd, args_prefix = get_echo_command()
    term.spawn(cmd, args=args_prefix + ["Hello, PTY!"])

    # Give it time to execute
    time.sleep(0.2)

    # Get the output
    output = term.content()
    print(f"Output:\n{output}")

    # Check exit status
    exit_code = term.try_wait()
    print(f"Exit code: {exit_code}\n")

    # Example 2: Run another command
    temp_dir = tempfile.gettempdir()
    print(f"Example 2: Running 'ls/dir {temp_dir}'")
    print("-" * 50)

    term2 = PtyTerminal(80, 24)
    cmd, args = get_ls_command(temp_dir)
    term2.spawn(cmd, args=args)

    time.sleep(0.3)

    output = term2.content()
    print(f"Output (first 500 chars):\n{output[:500]}")

    exit_code = term2.wait()  # Block until completion
    print(f"Exit code: {exit_code}\n")

    # Example 3: Working with cursor position
    print("Example 3: Cursor position tracking")
    print("-" * 50)

    term3 = PtyTerminal(80, 24)
    cmd, args_prefix = get_echo_command()
    term3.spawn(cmd, args=args_prefix + ["Line 1", "and", "Line 2"])

    time.sleep(0.2)

    cursor_pos = term3.cursor_position()
    print(f"Cursor position: col={cursor_pos[0]}, row={cursor_pos[1]}")
    print(f"Content:\n{term3.content()}\n")


if __name__ == "__main__":
    main()
