#!/usr/bin/env python3
"""
Terminal Streaming Demo

This example demonstrates how to stream a terminal session to web browsers
using the StreamingServer. It creates a PTY terminal, starts a shell, and
streams all terminal output in real-time via WebSocket.

Usage:
    python examples/streaming_demo.py [--port PORT] [--host HOST] [--theme THEME]

Then open examples/streaming_client.html in your web browser and connect to:
    ws://localhost:8080 (or your custom host:port)
"""

import argparse
import sys
import time
import select
from pathlib import Path

# Terminal color themes: (background, foreground, normal[8], bright[8])
THEMES: dict[str, dict[str, tuple[int, int, int] | list[tuple[int, int, int]]]] = {
    "iTerm2-dark": {
        "background": (0, 0, 0),
        "foreground": (255, 255, 255),
        "normal": [
            (0, 0, 0),        # black
            (201, 27, 0),     # red
            (0, 194, 0),      # green
            (199, 196, 0),    # yellow
            (2, 37, 199),     # blue
            (201, 48, 199),   # magenta
            (0, 197, 199),    # cyan
            (199, 199, 199),  # white
        ],
        "bright": [
            (104, 104, 104),  # bright black
            (255, 110, 103),  # bright red
            (95, 249, 103),   # bright green
            (254, 251, 103),  # bright yellow
            (104, 113, 255),  # bright blue
            (255, 118, 255),  # bright magenta
            (96, 253, 255),   # bright cyan
            (255, 255, 255),  # bright white
        ],
    },
    "monokai": {
        "background": (12, 12, 12),
        "foreground": (217, 217, 217),
        "normal": [
            (26, 26, 26),
            (244, 0, 95),
            (152, 224, 36),
            (253, 151, 31),
            (157, 101, 255),
            (244, 0, 95),
            (88, 209, 235),
            (196, 197, 181),
        ],
        "bright": [
            (98, 94, 76),
            (244, 0, 95),
            (152, 224, 36),
            (224, 213, 97),
            (157, 101, 255),
            (244, 0, 95),
            (88, 209, 235),
            (246, 246, 239),
        ],
    },
    "dracula": {
        "background": (40, 42, 54),
        "foreground": (248, 248, 242),
        "normal": [
            (33, 34, 44),
            (255, 85, 85),
            (80, 250, 123),
            (241, 250, 140),
            (189, 147, 249),
            (255, 121, 198),
            (139, 233, 253),
            (248, 248, 242),
        ],
        "bright": [
            (98, 114, 164),
            (255, 110, 110),
            (105, 255, 148),
            (255, 255, 165),
            (214, 172, 255),
            (255, 146, 223),
            (164, 255, 255),
            (255, 255, 255),
        ],
    },
    "solarized-dark": {
        "background": (0, 43, 54),
        "foreground": (131, 148, 150),
        "normal": [
            (7, 54, 66),
            (220, 50, 47),
            (133, 153, 0),
            (181, 137, 0),
            (38, 139, 210),
            (211, 54, 130),
            (42, 161, 152),
            (238, 232, 213),
        ],
        "bright": [
            (0, 43, 54),
            (203, 75, 22),
            (88, 110, 117),
            (101, 123, 131),
            (131, 148, 150),
            (108, 113, 196),
            (147, 161, 161),
            (253, 246, 227),
        ],
    },
}

# Add parent directory to path for development
sys.path.insert(0, str(Path(__file__).parent.parent))

try:
    import par_term_emu_core_rust as terminal_core
except ImportError:
    print("Error: par_term_emu_core_rust module not found.")
    print("Build the module with: uv run maturin develop --features streaming")
    sys.exit(1)


def apply_theme(pty_terminal: "terminal_core.PtyTerminal", theme_name: str) -> None:
    """Apply a color theme to the terminal."""
    if theme_name not in THEMES:
        print(f"Warning: Unknown theme '{theme_name}', using iTerm2-dark")
        theme_name = "iTerm2-dark"

    theme = THEMES[theme_name]
    bg = theme["background"]
    fg = theme["foreground"]
    normal = theme["normal"]
    bright = theme["bright"]

    # Set default colors
    pty_terminal.set_default_bg(*bg)  # type: ignore[arg-type]
    pty_terminal.set_default_fg(*fg)  # type: ignore[arg-type]

    # Set ANSI palette (0-7 normal, 8-15 bright)
    for i, color in enumerate(normal):  # type: ignore[arg-type]
        pty_terminal.set_ansi_palette_color(i, *color)
    for i, color in enumerate(bright):  # type: ignore[arg-type]
        pty_terminal.set_ansi_palette_color(i + 8, *color)

    print(f"Applied theme: {theme_name}")


def print_help():
    """Print available commands."""
    print("\nServer is running!")
    print("All PTY output is automatically streamed to connected clients.")
    print()
    print("Available commands:")
    print("  Ctrl+C   - Quit the streaming server")
    print("  s        - Show connection statistics")
    print()
    print("Connect via the web client and type commands in your shell!")
    print()


def main():
    parser = argparse.ArgumentParser(description='Terminal Streaming Demo')
    parser.add_argument('--host', default='127.0.0.1', help='Host to bind to (default: 127.0.0.1)')
    parser.add_argument('--port', type=int, default=8080, help='Port to bind to (default: 8080)')
    parser.add_argument('--cols', type=int, default=80, help='Terminal columns (default: 80)')
    parser.add_argument('--rows', type=int, default=24, help='Terminal rows (default: 24)')
    parser.add_argument('--scrollback', type=int, default=10000, help='Scrollback lines (default: 10000)')
    parser.add_argument('--shell', help='Shell to run (default: auto-detect)')
    parser.add_argument('--theme', default='iTerm2-dark', choices=list(THEMES.keys()),
                        help='Color theme (default: iTerm2-dark)')
    args = parser.parse_args()

    # Create PTY terminal
    print(f"Creating terminal ({args.cols}x{args.rows})...")
    pty_terminal = terminal_core.PtyTerminal(args.cols, args.rows, args.scrollback)

    # Apply theme
    apply_theme(pty_terminal, args.theme)

    # Start shell FIRST (so PTY writer is available for streaming server)
    print("Starting shell...")
    try:
        if args.shell:
            pty_terminal.spawn(args.shell, [])
        else:
            pty_terminal.spawn_shell()
    except Exception as e:
        print(f"Error starting shell: {e}")
        sys.exit(1)

    # Wait a moment for shell to start
    time.sleep(0.2)

    # Create streaming server (AFTER shell is spawned so PTY writer is available)
    addr = f"{args.host}:{args.port}"
    print(f"Creating streaming server on {addr}...")

    try:
        streaming_server = terminal_core.StreamingServer(pty_terminal, addr)
    except Exception as e:
        print(f"Error: Failed to create streaming server: {e}")
        print("\nMake sure the module was built with streaming support:")
        print("  uv run maturin develop --features streaming")
        sys.exit(1)

    # Start streaming server (non-blocking)
    print("Starting streaming server...")
    try:
        streaming_server.start()
    except Exception as e:
        print(f"Error starting streaming server: {e}")
        sys.exit(1)

    # Give the server a moment to start
    time.sleep(0.5)

    print(f"\n{'='*60}")
    print(f"  Terminal streaming server is running!")
    print(f"{'='*60}")
    print(f"\n  WebSocket URL: ws://{addr}")
    print(f"\n  1. Open examples/streaming_client.html in your browser or run `make dev-server` for enhanced demo")
    print(f"  2. Click 'Connect' to start streaming")
    print(f"\n{'='*60}\n")

    print_help()

    try:
        # Main event loop - handle user commands and resize requests
        while True:
            # Check if PTY process has exited
            if not pty_terminal.is_running():
                print("\nPTY process has exited")
                break

            # Poll for resize requests from clients
            try:
                resize_request = streaming_server.poll_resize()
                if resize_request is not None:
                    cols, rows = resize_request
                    print(f"\nResizing terminal to {cols}x{rows}")
                    pty_terminal.resize(cols, rows)
                    # Broadcast resize to all clients
                    streaming_server.send_resize(cols, rows)
            except Exception as e:
                print(f"Error handling resize: {e}")

            # Check for stdin input (non-blocking)
            # Note: select.select doesn't work on Windows for stdin
            # This is just for demonstration on Unix-like systems
            if sys.platform != 'win32':
                ready, _, _ = select.select([sys.stdin], [], [], 0)
                if ready:
                    try:
                        cmd = sys.stdin.readline().strip().lower()
                        if cmd == 's':
                            client_count = streaming_server.client_count()
                            print(f"Connected clients: {client_count}")
                            cols, rows = pty_terminal.size()
                            print(f"Terminal size: {cols}x{rows}")
                    except Exception as e:
                        print(f"Error: {e}")

            # Small sleep to prevent CPU spinning
            # Using 10ms instead of 100ms to minimize resize race condition
            time.sleep(0.01)

    except KeyboardInterrupt:
        print("\n\nReceived interrupt signal")

    finally:
        # Cleanup
        print("\nCleaning up...")

        # Shutdown streaming server
        try:
            streaming_server.shutdown("Server shutting down")
        except Exception as e:
            print(f"Error shutting down streaming server: {e}")

        # Stop PTY terminal
        if pty_terminal.is_running():
            try:
                pty_terminal.write(b"exit\n")
                time.sleep(0.5)
            except:
                pass

        print("Goodbye!")


if __name__ == '__main__':
    main()
