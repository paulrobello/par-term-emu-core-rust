#!/usr/bin/env python3
"""
Example demonstrating OSC 9 and OSC 777 notification support.

Notifications allow terminal applications to send desktop-style alerts
that can be displayed as toasts or system notifications.

Usage:
    python examples/notifications.py
"""

from par_term_emu_core_rust import Terminal


def main():
    """Demonstrate notification support"""
    term = Terminal(80, 24)

    print("=== Terminal Notification Support Demo ===\n")

    # OSC 9 - Simple notification (iTerm2/ConEmu style)
    print("1. OSC 9 - Simple Notification (no title)")
    print("   Sequence: ESC ] 9 ; message ST")
    term.process(b"\x1b]9;Build completed successfully!\x1b\\")

    if term.has_notifications():
        notifs = term.drain_notifications()
        for title, message in notifs:
            if title:
                print(f"   Received: [{title}] {message}")
            else:
                print(f"   Received: {message}")
    print()

    # OSC 777 - Structured notification (urxvt style)
    print("2. OSC 777 - Structured Notification (with title)")
    print("   Sequence: ESC ] 777 ; notify ; title ; message ST")
    term.process(b"\x1b]777;notify;Deployment Status;Application deployed to production\x1b\\")

    if term.has_notifications():
        notifs = term.drain_notifications()
        for title, message in notifs:
            print(f"   Received: [{title}] {message}")
    print()

    # Multiple notifications
    print("3. Multiple Notifications")
    term.process(b"\x1b]9;Task 1 complete\x1b\\")
    term.process(b"\x1b]777;notify;Warning;Low disk space\x1b\\")
    term.process(b"\x1b]9;Task 2 complete\x1b\\")
    term.process(b"\x1b]777;notify;Success;All tests passed\x1b\\")

    if term.has_notifications():
        notifs = term.drain_notifications()
        print(f"   Received {len(notifs)} notifications:")
        for i, (title, message) in enumerate(notifs, 1):
            if title:
                print(f"      {i}. [{title}] {message}")
            else:
                print(f"      {i}. {message}")
    print()

    # Unicode and emoji support
    print("4. Unicode and Emoji Support")
    term.process(b"\x1b]777;notify;\xf0\x9f\x8e\x89 Success;\xe2\x9c\x85 Operation completed!\x1b\\")

    if term.has_notifications():
        notifs = term.drain_notifications()
        for title, message in notifs:
            print(f"   Received: [{title}] {message}")
    print()

    # Practical use cases
    print("=== Practical Use Cases ===\n")

    print("Use Case 1: Long-running commands")
    print("   Command: sleep 300 && echo '\\e]9;Command finished\\e\\\\'")
    print("   Notifies when a long-running command completes\n")

    print("Use Case 2: Build systems")
    print("   Build script can send notifications on success/failure:")
    print("   echo '\\e]777;notify;Build Status;Build succeeded\\e\\\\'")
    print("   echo '\\e]777;notify;Build Error;Build failed\\e\\\\'\n")

    print("Use Case 3: Monitoring scripts")
    print("   Watch for events and send alerts:")
    print("   echo '\\e]9;Disk usage at 90%\\e\\\\'")
    print("   echo '\\e]777;notify;Security Alert;Suspicious activity detected\\e\\\\'\n")

    # Shell functions
    print("=== Shell Integration ===\n")
    print("Add these functions to your ~/.bashrc or ~/.zshrc:\n")

    print("Bash/Zsh:")
    print('  notify() { echo -e "\\e]9;$*\\e\\\\"; }')
    print('  notify_with_title() { echo -e "\\e]777;notify;$1;$2\\e\\\\"; }\n')

    print("Usage:")
    print("  # Simple notification")
    print("  notify 'Task completed'")
    print()
    print("  # Notification with title")
    print("  notify_with_title 'Build Status' 'Build succeeded'")
    print()

    # API Summary
    print("\n=== Python API Summary ===\n")
    print("Terminal Methods:")
    print("  • has_notifications() -> bool")
    print("    Check if there are pending notifications\n")
    print("  • drain_notifications() -> List[Tuple[str, str]]")
    print("    Get all pending notifications as [(title, message), ...]")
    print("    Clears the notification queue\n")
    print("  • take_notifications() -> List[Tuple[str, str]]")
    print("    Alias for drain_notifications()\n")

    print("\nNotes:")
    print("  • OSC 9 notifications have empty title string")
    print("  • OSC 777 notifications can have custom titles")
    print("  • Notifications are queued and can be polled")
    print("  • Supports Unicode and emoji characters")
    print("  • Compatible with iTerm2, ConEmu, and urxvt")


if __name__ == "__main__":
    main()
