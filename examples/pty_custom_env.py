#!/usr/bin/env python3
"""
Custom Environment and Working Directory Example

This example demonstrates:
- Setting custom environment variables
- Setting working directory
- Verifying environment is inherited
"""

import time
from par_term_emu_core_rust import PtyTerminal


def test_environment_variables():
    """Test custom environment variables"""
    print("=== Custom Environment Variables ===\n")

    term = PtyTerminal(80, 24)

    # Spawn shell with custom environment
    print("Setting environment variables:")
    print("  MY_VAR=HelloWorld")
    print("  DEBUG_MODE=enabled")
    print("  APP_VERSION=1.0.0\n")

    term.spawn(
        "/bin/sh",
        args=[
            "-c",
            "echo MY_VAR=$MY_VAR && echo DEBUG_MODE=$DEBUG_MODE && echo APP_VERSION=$APP_VERSION",
        ],
        env={
            "MY_VAR": "HelloWorld",
            "DEBUG_MODE": "enabled",
            "APP_VERSION": "1.0.0",
        },
    )

    time.sleep(0.3)

    output = term.content()
    print("Output:")
    print("-" * 50)
    print(output)

    exit_code = term.wait()
    print(f"Exit code: {exit_code}\n")


def test_working_directory():
    """Test custom working directory"""
    print("=== Custom Working Directory ===\n")

    term = PtyTerminal(80, 24)

    # Spawn command with custom working directory
    print("Running 'pwd' with cwd=/tmp\n")

    term.spawn("/bin/pwd", cwd="/tmp")

    time.sleep(0.2)

    output = term.content()
    print("Output:")
    print("-" * 50)
    print(output)

    if "/tmp" in output:
        print("✓ Working directory correctly set to /tmp\n")
    else:
        print("✗ Working directory not set correctly\n")

    term.wait()


def test_inherited_environment():
    """Test that parent environment is inherited"""
    print("=== Inherited Environment ===\n")

    import os

    # Set an environment variable in parent
    os.environ["PARENT_VAR"] = "from_parent"

    term = PtyTerminal(80, 24)

    print("Parent process set PARENT_VAR=from_parent")
    print("Checking if child process inherits it...\n")

    term.spawn("/bin/sh", args=["-c", "echo PARENT_VAR=$PARENT_VAR"])

    time.sleep(0.2)

    output = term.content()
    print("Output:")
    print("-" * 50)
    print(output)

    if "from_parent" in output:
        print("✓ Environment correctly inherited from parent\n")
    else:
        print("✗ Environment not inherited\n")

    term.wait()


def test_terminal_environment():
    """Test that terminal-specific variables are set"""
    print("=== Terminal Environment Variables ===\n")

    term = PtyTerminal(80, 24)

    print("Checking terminal-specific environment variables:\n")

    term.spawn(
        "/bin/sh",
        args=[
            "-c",
            "echo TERM=$TERM && echo COLORTERM=$COLORTERM && echo LINES=$LINES && echo COLUMNS=$COLUMNS",
        ],
    )

    time.sleep(0.3)

    output = term.content()
    print("Output:")
    print("-" * 50)
    print(output)

    # Check expected values
    checks = [
        ("TERM=xterm-256color", "TERM set to xterm-256color"),
        ("COLORTERM=truecolor", "COLORTERM set to truecolor"),
        ("LINES=24", "LINES set to 24"),
        ("COLUMNS=80", "COLUMNS set to 80"),
    ]

    print("\nValidation:")
    for expected, description in checks:
        if expected in output:
            print(f"✓ {description}")
        else:
            print(f"✗ {description} - NOT FOUND")

    term.wait()


def test_env_override():
    """Test that custom env vars override inherited ones"""
    print("\n=== Environment Variable Override ===\n")

    import os

    # Set an environment variable in parent
    os.environ["OVERRIDE_TEST"] = "original_value"

    term = PtyTerminal(80, 24)

    print("Parent has: OVERRIDE_TEST=original_value")
    print("Child sets: OVERRIDE_TEST=new_value\n")

    term.spawn(
        "/bin/sh",
        args=["-c", "echo OVERRIDE_TEST=$OVERRIDE_TEST"],
        env={"OVERRIDE_TEST": "new_value"},
    )

    time.sleep(0.2)

    output = term.content()
    print("Output:")
    print("-" * 50)
    print(output)

    if "new_value" in output:
        print("✓ Custom environment correctly overrides parent\n")
    else:
        print("✗ Override did not work\n")

    term.wait()


def main():
    test_environment_variables()
    test_working_directory()
    test_inherited_environment()
    test_terminal_environment()
    test_env_override()


if __name__ == "__main__":
    main()
