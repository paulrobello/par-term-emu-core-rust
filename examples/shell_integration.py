#!/usr/bin/env python3
"""
Shell integration (OSC 133) demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 30)

    print("=== Shell Integration (OSC 133) Demo ===\n")

    # OSC 133 is used by modern terminals (iTerm2, VSCode, etc.)
    # to mark different parts of the shell prompt and command execution

    # Marker A: Start of prompt
    print("1. Prompt Start (Marker A)")
    term.process_str("\x1b]133;A\x07")
    state = term.shell_integration_state()
    print(f"   State: {state}")
    print(f"   In prompt: {state.in_prompt}")
    print()

    term.process_str("user@host:~/project$ ")

    # Marker B: Start of command input
    print("2. Command Start (Marker B)")
    term.process_str("\x1b]133;B\x07")
    state = term.shell_integration_state()
    print(f"   State: {state}")
    print(f"   In command input: {state.in_command_input}")
    print()

    term.process_str("ls -la")

    # Marker C: Command executed (output starts)
    print("3. Command Executed (Marker C)")
    term.process_str("\x1b]133;C\x07\n")
    state = term.shell_integration_state()
    print(f"   State: {state}")
    print(f"   In command output: {state.in_command_output}")
    print()

    # Command output
    term.process_str("total 48\n")
    term.process_str("drwxr-xr-x  5 user user 4096 Nov  6 10:00 .\n")
    term.process_str("drwxr-xr-x 10 user user 4096 Nov  6 09:00 ..\n")
    term.process_str("-rw-r--r--  1 user user  123 Nov  6 10:00 file.txt\n")

    # Marker D: Command finished (with exit code)
    print("4. Command Finished (Marker D) with exit code 0")
    term.process_str("\x1b]133;D;0\x07")
    state = term.shell_integration_state()
    print(f"   State: {state}")
    print(f"   Last exit code: {state.last_exit_code}")
    print()

    # Show the terminal content
    print("Terminal content:")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print()

    # Example with a failed command
    print("=== Example with Failed Command ===\n")

    # Next prompt
    term.process_str("\x1b]133;A\x07")
    term.process_str("user@host:~/project$ ")

    term.process_str("\x1b]133;B\x07")
    term.process_str("cat nonexistent.txt")

    term.process_str("\x1b]133;C\x07\n")
    term.process_str("cat: nonexistent.txt: No such file or directory\n")

    # Exit code 1 (error)
    term.process_str("\x1b]133;D;1\x07")
    state = term.shell_integration_state()
    print(f"Command failed with exit code: {state.last_exit_code}")
    print()

    # Current working directory tracking
    print("=== Working Directory Tracking (OSC 7) ===\n")

    term.process_str("\x1b]7;/home/user/project\x07")
    state = term.shell_integration_state()
    print(f"Current directory: {state.cwd}")
    print()

    term.process_str("\x1b]133;A\x07")
    term.process_str("user@host:~/project$ ")

    term.process_str("\x1b]133;B\x07")
    term.process_str("cd /var/log")

    term.process_str("\x1b]133;C\x07\n")
    term.process_str("\x1b]133;D;0\x07")

    # Update directory
    term.process_str("\x1b]7;/var/log\x07")
    state = term.shell_integration_state()
    print(f"Changed directory to: {state.cwd}")
    print()

    # Benefits of shell integration
    print("=== Benefits of Shell Integration ===\n")

    print("1. Jump to Previous/Next Prompt:")
    print("   - Terminals can provide keyboard shortcuts to jump between prompts")
    print("   - Easy navigation of long command history")
    print()

    print("2. Command Status Indicators:")
    print("   - Visual markers for successful/failed commands")
    print("   - Quick identification of errors in history")
    print()

    print("3. Command Rerun:")
    print("   - Right-click on command to rerun")
    print("   - Copy command without selecting")
    print()

    print("4. Smart Selection:")
    print("   - Double-click to select command")
    print("   - Triple-click to select output")
    print()

    print("5. Directory Tracking:")
    print("   - Terminal knows current directory")
    print("   - Open new tab in same directory")
    print("   - Display directory in tab title")
    print()

    print("6. Command Duration:")
    print("   - Terminal can measure command execution time")
    print("   - Show timing information in prompt")
    print()

    # Example: Complete shell session
    print("=== Complete Shell Session Example ===\n")

    term.reset()

    def run_command(cmd, output, exit_code=0):
        """Simulate running a command with shell integration"""
        term.process_str("\x1b]133;A\x07")  # Prompt start
        term.process_str("user@host:~$ ")

        term.process_str("\x1b]133;B\x07")  # Command start
        term.process_str(cmd)

        term.process_str("\x1b]133;C\x07\n")  # Command executed
        term.process_str(output)

        term.process_str(f"\x1b]133;D;{exit_code}\x07")  # Command finished

    # Session
    term.process_str("\x1b]7;/home/user\x07")  # Set CWD

    run_command("echo 'Hello, World!'", "Hello, World!\n", 0)
    run_command("pwd", "/home/user\n", 0)
    run_command(
        "ls nonexistent",
        "ls: cannot access 'nonexistent': No such file or directory\n",
        2,
    )
    run_command("date", "Wed Nov  6 10:30:00 UTC 2024\n", 0)

    print("Shell session with integration markers:")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)

    # Final state
    state = term.shell_integration_state()
    print("\nFinal state:")
    print(f"  Current directory: {state.cwd}")
    print(f"  Last exit code: {state.last_exit_code}")


if __name__ == "__main__":
    main()
