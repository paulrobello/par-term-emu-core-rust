#!/usr/bin/env python3
"""
Bracketed paste mode demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 24)

    print("=== Bracketed Paste Mode Demo ===\n")

    # Initially bracketed paste is off
    print(f"Bracketed paste enabled: {term.bracketed_paste()}")
    print()

    # Without bracketed paste
    print("Without bracketed paste mode:")
    print("If you paste text with newlines, each line is executed immediately.")
    print("This can be dangerous for commands!\n")

    term.process_str("$ echo 'first command'\n")
    term.process_str("$ rm -rf /  # This would execute immediately if pasted!\n")
    term.process_str("$ echo 'second command'\n")

    print_terminal_content(term, show_colors=True)
    print()

    # Enable bracketed paste
    print("Enabling bracketed paste mode...")
    term.process_str("\x1b[?2004h")
    print(f"Bracketed paste enabled: {term.bracketed_paste()}")
    print()

    # Get the paste sequences
    paste_start = term.get_paste_start()
    paste_end = term.get_paste_end()

    print(f"Paste start sequence: {paste_start!r}")
    print(f"Paste end sequence: {paste_end!r}")
    print()

    # Simulate a paste operation
    print("With bracketed paste mode:")
    print("Pasted text is bracketed with special sequences,")
    print("allowing the shell to handle it safely.\n")

    term.reset()
    term.process_str("$ ")  # Prompt

    # Application receives paste start
    print("Simulating paste operation:")
    print(f"1. Terminal sends: {paste_start!r}")

    # Then the pasted content
    pasted_content = "echo 'line 1'\necho 'line 2'\necho 'line 3'"
    print(f"2. Pasted content: {pasted_content!r}")

    # Then paste end
    print(f"3. Terminal sends: {paste_end!r}")
    print()

    # The application can now handle this as a paste, not individual keystrokes
    print("The shell receives:")
    print(f"  {paste_start!r}")
    print("  + pasted content")
    print(f"  + {paste_end!r}")
    print()
    print("It knows this is a paste and can:")
    print("  • Display it as a multiline buffer")
    print("  • Not execute each line immediately")
    print("  • Allow the user to review before executing")
    print("  • Perform syntax highlighting")
    print()

    # Example with actual terminal content
    term.process_str("\x1b[?2004h")  # Enable bracketed paste

    # Simulating what the terminal would send
    term.process_str("$ # Pasting multi-line command...\n")

    print("Example: Pasting a multi-line script")
    print("-" * 80)

    # The paste indicators would be sent but not displayed
    script = """for i in {1..5}; do
    echo "Line $i"
    sleep 0.1
done"""

    # With bracketed paste, the shell would receive:
    #   \x1b[200~ + script content + \x1b[201~
    # And could display it without executing

    term.process_str("# Multi-line paste received (not executed):\n")
    for line in script.split("\n"):
        term.process_str(f"  {line}\n")

    term.process_str("# Press Enter to execute, or Ctrl+C to cancel\n")

    print_terminal_content(term, show_colors=True)
    print("-" * 80)
    print()

    # Disable bracketed paste
    term.process_str("\x1b[?2004l")
    print(f"Bracketed paste after disable: {term.bracketed_paste()}")
    print()

    # Practical benefits
    print("=== Benefits of Bracketed Paste ===\n")
    print("1. Security: Prevents accidental command execution")
    print("   - Pasting 'rm -rf /' won't execute immediately")
    print("   - User can review before running")
    print()
    print("2. Multi-line editing:")
    print("   - Paste functions/loops without line-by-line execution")
    print("   - Shell can treat paste as a single input")
    print()
    print("3. Better UX:")
    print("   - Syntax highlighting for pasted code")
    print("   - Indentation preservation")
    print("   - History management (single entry, not per line)")
    print()
    print("4. Special character handling:")
    print("   - Tabs and other special chars preserved")
    print("   - No unwanted expansions or substitutions")


if __name__ == "__main__":
    main()
