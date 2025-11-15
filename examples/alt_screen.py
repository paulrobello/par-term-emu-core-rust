#!/usr/bin/env python3
"""
Alternate screen buffer demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 24)

    print("=== Alternate Screen Demo ===\n")

    # Write to primary screen
    term.process_str("This is the PRIMARY screen.\n")
    term.process_str("You're reading the main terminal buffer.\n")
    term.process_str("This content will be preserved when we switch screens.\n\n")
    for i in range(1, 6):
        term.process_str(f"Line {i} on primary screen\n")

    print("Primary screen content:")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print(f"Alt screen active: {term.is_alt_screen_active()}")
    print()

    # Switch to alternate screen
    print("Switching to alternate screen...")
    term.process_str("\x1b[?1049h")  # Enable alt screen

    print(f"Alt screen active: {term.is_alt_screen_active()}")
    print()

    # The alternate screen should be clear
    term.process_str("╔════════════════════════════════════════════════════════════╗\n")
    term.process_str("║              ALTERNATE SCREEN BUFFER                       ║\n")
    term.process_str("╠════════════════════════════════════════════════════════════╣\n")
    term.process_str("║                                                            ║\n")
    term.process_str("║  This is useful for full-screen applications like:        ║\n")
    term.process_str("║  • Text editors (vim, nano, emacs)                         ║\n")
    term.process_str("║  • Terminal multiplexers (tmux, screen)                    ║\n")
    term.process_str("║  • Interactive programs (less, man pages)                  ║\n")
    term.process_str("║  • TUI applications                                        ║\n")
    term.process_str("║                                                            ║\n")
    term.process_str("║  The primary screen content is preserved!                 ║\n")
    term.process_str("║                                                            ║\n")
    term.process_str("╚════════════════════════════════════════════════════════════╝\n")

    print("Alternate screen content:")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print()

    # Switch back to primary screen
    print("Switching back to primary screen...")
    term.process_str("\x1b[?1049l")  # Disable alt screen

    print(f"Alt screen active: {term.is_alt_screen_active()}")
    print()

    # The primary screen content should be restored
    print("Primary screen content (restored):")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print()

    # Demonstrate typical use case: a menu program
    print("=== Simulating a TUI Application ===\n")

    # Save primary screen content
    term.reset()
    term.process_str("$ ls\n")
    term.process_str("file1.txt  file2.txt  file3.txt\n")
    term.process_str("$ cat file1.txt\n")
    term.process_str("Some important content...\n")
    term.process_str("$ ./menu_app\n")

    term.content()

    # Enter alt screen for the app
    term.process_str("\x1b[?1049h")

    # Draw a menu
    term.process_str("\x1b[2J\x1b[H")  # Clear and home
    term.process_str("\x1b[1;34m┌─── Main Menu ───┐\x1b[0m\n")
    term.process_str("\x1b[1;34m│\x1b[0m 1. Option A    \x1b[1;34m│\x1b[0m\n")
    term.process_str("\x1b[1;34m│\x1b[0m 2. Option B    \x1b[1;34m│\x1b[0m\n")
    term.process_str("\x1b[1;34m│\x1b[0m 3. Exit        \x1b[1;34m│\x1b[0m\n")
    term.process_str("\x1b[1;34m└─────────────────┘\x1b[0m\n")
    term.process_str("\nSelection: 3\n")

    print("Menu application running (alt screen):")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print()

    # Exit the app and return to primary screen
    term.process_str("\x1b[?1049l")

    term.process_str("$ # Back to shell\n")

    print("After exiting menu app (primary screen restored):")
    print("-" * 80)
    print_terminal_content(term, show_colors=True)
    print("-" * 80)


if __name__ == "__main__":
    main()
