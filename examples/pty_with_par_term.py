"""Example demonstrating PTY usage with par-term terminfo.

This example shows how to use the custom par-term terminfo definition
for optimal terminal compatibility.

Prerequisites:
1. Install the terminfo: ./terminfo/install.sh
2. Run this example: uv run python examples/pty_with_par_term.py

The par-term terminfo provides full support for:
- 24-bit true color (RGB)
- Sixel graphics
- Bracketed paste
- Mouse tracking
- Focus tracking
- Underline styles
- And more VT420 features
"""

import time
from par_term_emu_core_rust import PtyTerminal


def check_terminfo_installed() -> bool:
    """Check if par-term terminfo is installed."""
    import subprocess

    try:
        result = subprocess.run(
            ["infocmp", "par-term"],
            capture_output=True,
            text=True,
            check=False,
        )
        return result.returncode == 0
    except FileNotFoundError:
        print("Warning: 'infocmp' command not found")
        return False


def main() -> None:
    """Demonstrate using par-term with PTY."""
    # Check if terminfo is installed
    terminfo_available = check_terminfo_installed()

    if terminfo_available:
        print("✓ par-term terminfo is installed")
        term_type = "par-term"
    else:
        print("⚠ par-term terminfo not found, using xterm-256color")
        print("  Install with: ./terminfo/install.sh")
        term_type = "xterm-256color"

    print(f"\nUsing TERM={term_type}")
    print("=" * 60)

    # Create a PTY terminal with par-term environment
    with PtyTerminal(80, 24) as term:
        # Spawn a shell with par-term terminfo
        term.spawn_shell(
            env={
                "TERM": term_type,
                "COLORTERM": "truecolor",
            }
        )

        # Give shell time to initialize
        time.sleep(0.1)

        # Test terminal capabilities
        print("\nTesting terminal capabilities:")
        print("-" * 60)

        # 1. Test color support
        term.write_str("tput colors\n")
        time.sleep(0.1)

        # 2. Test true color (if par-term is available)
        if terminfo_available:
            print("\nTesting true color support:")
            term.write_str('printf "\\e[38;2;255;128;0mOrange text\\e[0m\\n"\n')
            time.sleep(0.1)

            # Test underline styles
            print("\nTesting underline styles:")
            term.write_str('printf "\\e[4:3mCurly underline\\e[0m\\n"\n')
            time.sleep(0.1)

        # 3. Test bracketed paste capability
        print("\nTesting bracketed paste:")
        term.write_str("tput BD 2>/dev/null && echo 'Bracketed paste: ON'\n")
        time.sleep(0.1)

        # 4. Show terminal type
        print("\nTerminal identification:")
        term.write_str("echo $TERM\n")
        term.write_str("echo $COLORTERM\n")
        time.sleep(0.1)

        # 5. Display some formatted output
        print("\nFormatted output test:")
        term.write_str("clear\n")
        time.sleep(0.05)
        term.write_str(
            'printf "\\e[1;34m╔════════════════════════════╗\\e[0m\\n"\n'
        )
        term.write_str(
            'printf "\\e[1;34m║\\e[0m \\e[1;32mPAR Terminal Emulator\\e[0m  \\e[1;34m║\\e[0m\\n"\n'
        )
        term.write_str(
            'printf "\\e[1;34m║\\e[0m \\e[36mFull VT420 Support\\e[0m     \\e[1;34m║\\e[0m\\n"\n'
        )
        term.write_str(
            'printf "\\e[1;34m╚════════════════════════════╝\\e[0m\\n"\n'
        )
        time.sleep(0.2)

        # Get and display the terminal content
        print("\n" + "=" * 60)
        print("Terminal Output:")
        print("=" * 60)
        content = term.content()
        print(content)

        # Show cursor position
        col, row = term.cursor_position()
        print("=" * 60)
        print(f"Cursor position: ({col}, {row})")

        # Clean exit
        term.write_str("exit\n")
        time.sleep(0.1)

    print("\n✓ PTY session completed")


if __name__ == "__main__":
    main()
