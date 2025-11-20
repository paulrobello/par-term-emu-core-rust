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
import threading
from pathlib import Path

# Add parent directory to path for development
sys.path.insert(0, str(Path(__file__).parent.parent))

try:
    import par_term_emu_core_rust as terminal_core
except ImportError:
    print("Error: par_term_emu_core_rust module not found.")
    print("Build the module with: uv run maturin develop --features streaming")
    sys.exit(1)


def read_pty_output(pty_terminal, streaming_server, stop_event):
    """
    Read output from PTY and forward it to the streaming server.

    This runs in a separate thread and continuously reads from the PTY,
    forwarding all output to connected web clients.
    """
    print("Starting PTY output reader thread...")

    try:
        while not stop_event.is_set() and pty_terminal.is_running():
            # Read output from PTY (non-blocking with timeout)
            output = pty_terminal.read_output(timeout_ms=100)

            if output:
                # Forward output to all connected streaming clients
                try:
                    streaming_server.send_output(output)
                except Exception as e:
                    print(f"Error sending output to streaming server: {e}")

            # Small sleep to prevent CPU spinning
            time.sleep(0.01)

    except Exception as e:
        print(f"PTY reader thread error: {e}")
    finally:
        print("PTY reader thread stopped")


def handle_user_commands(pty_terminal, streaming_server):
    """
    Handle user commands from stdin (for demonstration).

    In a real application, input would come from the web client.
    """
    print("\nCommands:")
    print("  q, quit  - Quit the streaming server")
    print("  s, stats - Show connection statistics")
    print("  c, clear - Clear the terminal")
    print("  b, bell  - Send a bell event")
    print()

    while True:
        try:
            cmd = input("> ").strip().lower()

            if cmd in ('q', 'quit'):
                print("Shutting down...")
                return False

            elif cmd in ('s', 'stats'):
                client_count = streaming_server.client_count()
                print(f"Connected clients: {client_count}")

                # Get terminal stats
                if pty_terminal.is_running():
                    cols, rows = pty_terminal.get_size()
                    print(f"Terminal size: {cols}x{rows}")

            elif cmd in ('c', 'clear'):
                streaming_server.send_output("\x1b[2J\x1b[H")
                print("Sent clear screen command")

            elif cmd in ('b', 'bell'):
                streaming_server.send_bell()
                print("Sent bell event")

            else:
                print(f"Unknown command: {cmd}")

        except KeyboardInterrupt:
            print("\nShutting down...")
            return False
        except EOFError:
            return False
        except Exception as e:
            print(f"Error: {e}")

    return True


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

    # Start PTY output reader thread
    stop_event = threading.Event()
    reader_thread = threading.Thread(
        target=read_pty_output,
        args=(pty_terminal, streaming_server, stop_event),
        daemon=True
    )
    reader_thread.start()

    # Wait a moment for shell to start
    time.sleep(0.2)

    try:
        # Main loop - handle user commands
        handle_user_commands(pty_terminal, streaming_server)

    except KeyboardInterrupt:
        print("\n\nReceived interrupt signal")

    finally:
        # Cleanup
        print("\nCleaning up...")
        stop_event.set()

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

        # Wait for reader thread
        reader_thread.join(timeout=2.0)

        print("Goodbye!")


if __name__ == '__main__':
    main()
