#!/usr/bin/env python3
"""
Terminal Streaming Demo

This example demonstrates how to stream a terminal session to web browsers
using the StreamingServer. It creates a PTY terminal, starts a shell, and
streams all terminal output in real-time via WebSocket.

Usage:
    python examples/streaming_demo.py [--port PORT] [--host HOST]

Then open examples/streaming_client.html in your web browser and connect to:
    ws://localhost:8080 (or your custom host:port)
"""

import argparse
import sys
import time
import select
from pathlib import Path

# Add parent directory to path for development
sys.path.insert(0, str(Path(__file__).parent.parent))

try:
    import par_term_emu_core_rust as terminal_core
except ImportError:
    print("Error: par_term_emu_core_rust module not found.")
    print("Build the module with: uv run maturin develop --features streaming")
    sys.exit(1)


def process_pty_output(pty_terminal, streaming_server):
    """
    Read output from PTY and forward it to the streaming server.

    Returns True if the PTY is still running, False otherwise.
    """
    if not pty_terminal.is_running():
        return False

    # Read output from PTY (non-blocking with timeout)
    output = pty_terminal.read_output(timeout_ms=10)

    if output:
        # Forward output to all connected streaming clients
        try:
            streaming_server.send_output(output)
        except Exception as e:
            print(f"Error sending output to streaming server: {e}")

    return True


def print_help():
    """Print available commands."""
    print("\nAvailable commands:")
    print("  Ctrl+C   - Quit the streaming server")
    print("  s, stats - Show connection statistics (type 's' and press Enter)")
    print()
    print("The terminal is running. Connect via the web client to interact.")
    print()


def main():
    parser = argparse.ArgumentParser(description='Terminal Streaming Demo')
    parser.add_argument('--host', default='127.0.0.1', help='Host to bind to (default: 127.0.0.1)')
    parser.add_argument('--port', type=int, default=8080, help='Port to bind to (default: 8080)')
    parser.add_argument('--cols', type=int, default=80, help='Terminal columns (default: 80)')
    parser.add_argument('--rows', type=int, default=24, help='Terminal rows (default: 24)')
    parser.add_argument('--scrollback', type=int, default=10000, help='Scrollback lines (default: 10000)')
    parser.add_argument('--shell', help='Shell to run (default: auto-detect)')
    args = parser.parse_args()

    # Create PTY terminal
    print(f"Creating terminal ({args.cols}x{args.rows})...")
    pty_terminal = terminal_core.PtyTerminal(args.cols, args.rows, args.scrollback)

    # Create streaming server
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
    print(f"\n  1. Open examples/streaming_client.html in your browser")
    print(f"  2. Click 'Connect' to start streaming")
    print(f"\n{'='*60}\n")

    # Start shell
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

    print_help()

    try:
        # Main event loop - process PTY output and check for stdin input
        while True:
            # Process PTY output and forward to streaming server
            if not process_pty_output(pty_terminal, streaming_server):
                print("\nPTY process has exited")
                break

            # Check for stdin input (non-blocking)
            # Note: select.select doesn't work on Windows for stdin
            # This is just for demonstration on Unix-like systems
            if sys.platform != 'win32':
                ready, _, _ = select.select([sys.stdin], [], [], 0)
                if ready:
                    try:
                        cmd = sys.stdin.readline().strip().lower()
                        if cmd in ('s', 'stats'):
                            client_count = streaming_server.client_count()
                            print(f"Connected clients: {client_count}")
                            if pty_terminal.is_running():
                                cols, rows = pty_terminal.get_size()
                                print(f"Terminal size: {cols}x{rows}")
                    except:
                        pass

            # Small sleep to prevent CPU spinning
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
