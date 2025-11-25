#!/usr/bin/env python3
"""
Streaming Debug Tool

This tool helps debug the web terminal streaming gradient rendering issue by:
1. Running the streaming server with verbose size logging
2. Capturing exact terminal state before/after Rich runs
3. Comparing terminal emulator state vs raw output sent to xterm.js

Usage:
    python examples/streaming_debug.py [--port PORT] [--host HOST]

Then in the web terminal:
1. Run: python examples/gradient_test.py
2. Check the console output for size discrepancies
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


def main() -> None:
    """Run the streaming debug server."""
    parser = argparse.ArgumentParser(description="Streaming Debug Server")
    parser.add_argument(
        "--host", default="127.0.0.1", help="Host to bind to (default: 127.0.0.1)"
    )
    parser.add_argument(
        "--port", type=int, default=8080, help="Port to bind to (default: 8080)"
    )
    parser.add_argument(
        "--cols", type=int, default=80, help="Terminal columns (default: 80)"
    )
    parser.add_argument(
        "--rows", type=int, default=24, help="Terminal rows (default: 24)"
    )
    args = parser.parse_args()

    # Create PTY terminal
    print(f"[DEBUG] Creating terminal ({args.cols}x{args.rows})...")
    pty_terminal = terminal_core.PtyTerminal(args.cols, args.rows, 1000)

    # Log initial state
    print(f"[DEBUG] Initial PTY size: {pty_terminal.size()}")

    # Start shell
    print("[DEBUG] Starting shell...")
    try:
        pty_terminal.spawn_shell()
    except Exception as e:
        print(f"Error starting shell: {e}")
        sys.exit(1)

    time.sleep(0.2)

    # Create streaming server
    addr = f"{args.host}:{args.port}"
    print(f"[DEBUG] Creating streaming server on {addr}...")

    try:
        streaming_server = terminal_core.StreamingServer(pty_terminal, addr)
    except Exception as e:
        print(f"Error: Failed to create streaming server: {e}")
        sys.exit(1)

    # Start streaming server
    print("[DEBUG] Starting streaming server...")
    try:
        streaming_server.start()
    except Exception as e:
        print(f"Error starting streaming server: {e}")
        sys.exit(1)

    time.sleep(0.5)

    print(f"\n{'='*60}")
    print("  Streaming Debug Server Running")
    print(f"{'='*60}")
    print(f"\n  WebSocket URL: ws://{addr}")
    print("\n  Commands:")
    print("    s    - Show size info")
    print("    d    - Dump terminal state")
    print("    q    - Quit")
    print(f"\n{'='*60}\n")

    # Track resize events
    last_resize_time = 0.0
    resize_count = 0

    try:
        while True:
            # Check if PTY process has exited
            if not pty_terminal.is_running():
                print("\n[DEBUG] PTY process has exited")
                break

            # Poll for resize requests (FAST - 10ms instead of 100ms)
            try:
                resize_request = streaming_server.poll_resize()
                if resize_request is not None:
                    cols, rows = resize_request
                    resize_count += 1
                    now = time.time()
                    delta = now - last_resize_time if last_resize_time > 0 else 0
                    last_resize_time = now

                    print(
                        f"[RESIZE #{resize_count}] {cols}x{rows} "
                        f"(delta: {delta*1000:.1f}ms)"
                    )

                    # Resize immediately
                    old_cols, old_rows = pty_terminal.size()
                    pty_terminal.resize(cols, rows)
                    new_cols, new_rows = pty_terminal.size()

                    if old_cols != new_cols or old_rows != new_rows:
                        print(f"  PTY resized: {old_cols}x{old_rows} -> {new_cols}x{new_rows}")

                    # Broadcast resize to clients
                    streaming_server.send_resize(cols, rows)

            except Exception as e:
                print(f"[ERROR] Resize handling: {e}")

            # Check for stdin input (non-blocking)
            if sys.platform != "win32":
                ready, _, _ = select.select([sys.stdin], [], [], 0)
                if ready:
                    try:
                        cmd = sys.stdin.readline().strip().lower()
                        if cmd == "s":
                            cols, rows = pty_terminal.size()
                            print(f"[SIZE] PTY: {cols}x{rows}")
                            print(f"[SIZE] Resize events: {resize_count}")
                            print(f"[SIZE] Connected clients: {streaming_server.client_count()}")
                        elif cmd == "d":
                            cols, rows = pty_terminal.size()
                            print(f"\n[DUMP] Terminal size: {cols}x{rows}")
                            # Get visible screen styled output
                            styled = pty_terminal.export_visible_screen_styled()
                            print(f"[DUMP] Styled output length: {len(styled)} bytes")
                            # Show first line as hex for debugging
                            first_line = styled.split("\n")[0] if "\n" in styled else styled[:100]
                            hex_line = " ".join(f"{ord(c):02x}" for c in first_line[:50])
                            print(f"[DUMP] First 50 chars (hex): {hex_line}")
                        elif cmd == "q":
                            print("[DEBUG] Quitting...")
                            break
                    except Exception as e:
                        print(f"[ERROR] Command: {e}")

            # Smaller sleep for faster resize response
            time.sleep(0.01)  # 10ms instead of 100ms

    except KeyboardInterrupt:
        print("\n\n[DEBUG] Interrupted")

    finally:
        print("[DEBUG] Cleaning up...")
        try:
            streaming_server.shutdown("Debug server shutting down")
        except Exception as e:
            print(f"[ERROR] Shutdown: {e}")

        if pty_terminal.is_running():
            try:
                pty_terminal.write(b"exit\n")
                time.sleep(0.5)
            except Exception:
                pass

        print("[DEBUG] Done")


if __name__ == "__main__":
    main()
