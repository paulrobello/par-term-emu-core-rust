#!/usr/bin/env python3
"""
Event Loop Integration Example

This example demonstrates:
- Using update_generation() for efficient event loops
- Detecting when terminal content changes
- Minimizing unnecessary redraws
- Non-blocking update checking
"""

import sys
import time
from par_term_emu_core_rust import PtyTerminal


def efficient_event_loop():
    """Demonstrate efficient event loop using update tracking"""
    print("=== Efficient Event Loop ===\n")

    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Track the last generation we processed
    last_generation = term.update_generation()
    print(f"Initial generation: {last_generation}")

    # Send a command
    print("Sending command: echo 'Hello, World!'")
    term.write_str("echo 'Hello, World!'\n")

    # Event loop
    updates_detected = 0
    iterations = 0
    max_iterations = 50  # Limit iterations for demo

    print("\nEvent loop running...")
    print("(Checking for updates without blocking)\n")

    start_time = time.time()

    while iterations < max_iterations and term.is_running():
        iterations += 1

        # Check if there are updates (non-blocking)
        if term.has_updates_since(last_generation):
            # Content changed, update our view
            current_generation = term.update_generation()
            updates_detected += 1

            print(
                f"Update #{updates_detected} detected (generation: {current_generation})"
            )

            # In a real application, you would redraw here
            # For this demo, we'll just note that an update occurred
            last_generation = current_generation

        # Small sleep to simulate event loop iteration
        time.sleep(0.05)

    elapsed = time.time() - start_time

    print("\nLoop stats:")
    print(f"  Iterations: {iterations}")
    print(f"  Updates detected: {updates_detected}")
    print(f"  Elapsed time: {elapsed:.2f}s")
    print(f"  Final generation: {term.update_generation()}")

    # Show final content
    print("\nFinal terminal content:")
    print("-" * 60)
    content = term.content()
    lines = content.strip().split("\n")
    for line in lines[-15:]:  # Last 15 lines
        print(line)

    # Clean up
    term.write_str("exit\n")
    time.sleep(0.2)
    if term.is_running():
        term.kill()


def multiple_commands_tracking():
    """Track updates across multiple commands"""
    print("\n\n=== Multiple Commands with Update Tracking ===\n")

    term = PtyTerminal(80, 24)
    term.spawn_shell()
    time.sleep(0.5)

    term.update_generation()
    commands = ["ls", "pwd", "whoami", "date"]

    print("Sending commands and tracking updates:\n")

    for cmd in commands:
        print(f"Command: {cmd}")
        term.write_str(f"{cmd}\n")

        # Wait for updates
        start_gen = term.update_generation()
        max_wait = 1.0  # 1 second timeout
        start_time = time.time()

        while time.time() - start_time < max_wait:
            if term.has_updates_since(start_gen):
                break
            time.sleep(0.01)

        # Show generation change
        new_gen = term.update_generation()
        updates = new_gen - start_gen
        print(f"  Generation: {start_gen} -> {new_gen} ({updates} updates)")

        time.sleep(0.2)  # Give command time to finish

    # Clean up
    term.write_str("exit\n")
    time.sleep(0.2)
    if term.is_running():
        term.kill()


def detect_process_output():
    """Detect when a long-running process produces output"""
    print("\n\n=== Detecting Process Output ===\n")

    term = PtyTerminal(80, 24)

    # Run a command that produces output over time
    if sys.platform == "win32":
        print("Starting: for loop counting 1-5 with delays\n")
        term.spawn(
            "cmd.exe",
            args=[
                "/C",
                "for /L %i in (1,1,5) do @(echo %i && timeout /t 1 /nobreak >nul)",
            ],
        )
    else:
        print("Starting: for i in 1 2 3 4 5; do echo $i; sleep 0.5; done\n")
        term.spawn(
            "/bin/sh",
            args=["-c", "for i in 1 2 3 4 5; do echo $i; sleep 0.5; done"],
        )

    last_generation = term.update_generation()
    output_events = []

    print("Waiting for output...\n")

    while term.is_running() or term.has_updates_since(last_generation):
        if term.has_updates_since(last_generation):
            current_generation = term.update_generation()
            timestamp = time.time()

            # Get latest content
            content = term.content()
            lines = [line for line in content.strip().split("\n") if line]

            output_events.append(
                (timestamp, current_generation, lines[-1] if lines else "")
            )

            print(f"Output detected: generation {current_generation}")

            last_generation = current_generation

        time.sleep(0.1)

    print(f"\nTotal output events: {len(output_events)}")
    print("\nOutput timeline:")
    start_time = output_events[0][0] if output_events else time.time()
    for timestamp, generation, last_line in output_events:
        elapsed = timestamp - start_time
        print(f"  +{elapsed:.2f}s: gen={generation}, content='{last_line}'")

    exit_code = term.try_wait()
    print(f"\nProcess exited with code: {exit_code}")


def main():
    efficient_event_loop()
    multiple_commands_tracking()
    detect_process_output()


if __name__ == "__main__":
    main()
