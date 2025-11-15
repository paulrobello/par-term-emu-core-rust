#!/usr/bin/env python3
"""
Multiple Concurrent PTY Sessions Example

This example demonstrates:
- Running multiple PTY sessions concurrently
- Managing multiple processes
- Collecting output from different sessions
"""

import time
from par_term_emu_core_rust import PtyTerminal


def run_concurrent_commands():
    """Run multiple commands concurrently"""
    print("=== Multiple Concurrent Commands ===\n")

    # Create multiple terminals with different commands
    sessions = [
        (PtyTerminal(80, 24), "/bin/echo", ["Session 1: Hello"]),
        (PtyTerminal(80, 24), "/bin/echo", ["Session 2: World"]),
        (PtyTerminal(80, 24), "/bin/echo", ["Session 3: Concurrent"]),
        (PtyTerminal(80, 24), "/bin/echo", ["Session 4: PTY"]),
    ]

    # Spawn all commands
    print("Spawning 4 concurrent sessions...")
    for i, (term, cmd, args) in enumerate(sessions, 1):
        term.spawn(cmd, args=args)
        print(f"  Session {i} started")

    # Give them time to execute
    time.sleep(0.3)

    # Collect results
    print("\nResults:")
    print("=" * 60)
    for i, (term, cmd, args) in enumerate(sessions, 1):
        output = term.content().strip()
        exit_code = term.try_wait()
        print(f"Session {i}: {output} (exit: {exit_code})")

    print()


def run_parallel_shells():
    """Run multiple interactive shells"""
    print("=== Multiple Interactive Shells ===\n")

    # Create 3 shell sessions
    shells = []
    for i in range(3):
        term = PtyTerminal(80, 24)
        term.spawn_shell()
        shells.append((i + 1, term))

    # Give shells time to start
    time.sleep(0.5)

    print("Created 3 shell sessions\n")

    # Send different commands to each shell
    commands = [
        ("echo 'Shell 1: pwd'", "pwd"),
        ("echo 'Shell 2: whoami'", "whoami"),
        ("echo 'Shell 3: date'", "date"),
    ]

    print("Sending different commands to each shell:")
    for (shell_id, term), (desc, cmd) in zip(shells, commands):
        print(f"  Shell {shell_id}: {cmd}")
        term.write_str(f"{cmd}\n")

    # Give commands time to execute
    time.sleep(0.5)

    # Collect outputs
    print("\nOutputs:")
    print("=" * 60)
    for shell_id, term in shells:
        output = term.content()
        # Print last 10 lines
        lines = output.strip().split("\n")
        print(f"\nShell {shell_id}:")
        print("-" * 40)
        for line in lines[-10:]:
            print(line)

    # Clean up all shells
    print("\nCleaning up...")
    for shell_id, term in shells:
        term.write_str("exit\n")
        time.sleep(0.2)
        if term.is_running():
            term.kill()

    print("All shells terminated\n")


def run_long_running_processes():
    """Demonstrate managing long-running processes"""
    print("=== Long-Running Processes ===\n")

    # Start a few long-running processes
    processes = []

    # Process 1: Sleep for 2 seconds
    term1 = PtyTerminal(80, 24)
    term1.spawn("/bin/sleep", args=["2"])
    processes.append(("sleep 2", term1))

    # Process 2: Count slowly
    term2 = PtyTerminal(80, 24)
    term2.spawn("/bin/sh", args=["-c", "for i in 1 2 3; do echo $i; sleep 0.5; done"])
    processes.append(("counter", term2))

    # Process 3: Another sleep
    term3 = PtyTerminal(80, 24)
    term3.spawn("/bin/sleep", args=["1"])
    processes.append(("sleep 1", term3))

    print("Started 3 long-running processes")

    # Poll all processes until they complete
    start_time = time.time()
    while any(term.is_running() for _, term in processes):
        time.sleep(0.2)

        for name, term in processes:
            if term.is_running():
                # Check if it completed
                exit_code = term.try_wait()
                if exit_code is not None:
                    elapsed = time.time() - start_time
                    print(
                        f"  {name:12} completed (exit: {exit_code}) after {elapsed:.1f}s"
                    )

    elapsed = time.time() - start_time
    print(f"\nAll processes completed after {elapsed:.1f}s")

    # Show output from the counter
    output = processes[1][1].content()
    print("\nCounter output:")
    print(output)


def stress_test_many_sessions():
    """Create many sessions to test resource handling"""
    print("\n=== Stress Test: Many Sessions ===\n")

    num_sessions = 10
    print(f"Creating {num_sessions} concurrent sessions...")

    sessions = []
    for i in range(num_sessions):
        term = PtyTerminal(80, 24)
        term.spawn("/bin/echo", args=[f"Session {i + 1}"])
        sessions.append(term)

    time.sleep(0.5)

    # Check they all completed
    completed = sum(1 for term in sessions if term.try_wait() is not None)
    print(f"Completed: {completed}/{num_sessions}")

    if completed == num_sessions:
        print("✓ All sessions completed successfully\n")
    else:
        print(f"✗ Only {completed} sessions completed\n")


def main():
    run_concurrent_commands()
    run_parallel_shells()
    run_long_running_processes()
    stress_test_many_sessions()


if __name__ == "__main__":
    main()
