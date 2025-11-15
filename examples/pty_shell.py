#!/usr/bin/env python3
"""
Interactive Shell Example

This example demonstrates:
- Spawning an interactive shell
- Sending commands
- Reading output
- Proper cleanup
"""

import time
from par_term_emu_core_rust import PtyTerminal


def run_shell_commands():
    """Run a series of commands in a shell session"""
    print("=== Interactive Shell Example ===\n")

    # Create terminal and spawn shell using context manager
    with PtyTerminal(80, 24) as term:
        print(f"Spawning shell: {PtyTerminal.get_default_shell()}")
        term.spawn_shell()

        # Give shell time to start
        time.sleep(0.5)

        print("\n" + "=" * 60)
        print("Running: echo 'Hello from shell'")
        print("=" * 60)

        # Send first command
        term.write_str("echo 'Hello from shell'\n")
        time.sleep(0.3)

        # Get output
        output = term.content()
        print(output)

        print("\n" + "=" * 60)
        print("Running: pwd")
        print("=" * 60)

        # Send second command
        term.write_str("pwd\n")
        time.sleep(0.3)

        output = term.content()
        print(output)

        print("\n" + "=" * 60)
        print("Running: whoami")
        print("=" * 60)

        # Send third command
        term.write_str("whoami\n")
        time.sleep(0.3)

        output = term.content()
        print(output)

        print("\n" + "=" * 60)
        print("Running: exit")
        print("=" * 60)

        # Exit the shell
        term.write_str("exit\n")
        time.sleep(0.3)

        # Check if process has exited
        exit_code = term.try_wait()
        if exit_code is not None:
            print(f"\nShell exited with code: {exit_code}")
        else:
            print("\nShell still running, killing...")
            term.kill()

    # Context manager automatically cleans up
    print("\nContext manager cleaned up the shell process")


def run_multiline_command():
    """Demonstrate running multiline commands"""
    print("\n\n=== Multiline Command Example ===\n")

    term = PtyTerminal(80, 24)
    term.spawn_shell()
    time.sleep(0.5)

    print("Running a for loop in bash:")
    print("-" * 50)

    # Send a multiline command
    term.write_str("for i in 1 2 3; do\n")
    time.sleep(0.1)
    term.write_str("  echo 'Number: '$i\n")
    time.sleep(0.1)
    term.write_str("done\n")
    time.sleep(0.5)

    output = term.content()
    print(output)

    # Clean up
    term.write_str("exit\n")
    time.sleep(0.2)
    term.kill()


def main():
    run_shell_commands()
    run_multiline_command()


if __name__ == "__main__":
    main()
