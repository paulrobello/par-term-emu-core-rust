# Security Considerations for PTY Usage

This document outlines security considerations when using the PTY functionality in par-term-emu.

## API Overview

The `PtyTerminal.spawn()` method signature:

```python
term.spawn(
    command: str,                      # Required: command to execute
    args: list[str] | None = None,     # Optional: list of command arguments
    env: dict[str, str] | None = None, # Optional: environment variables (overrides inherited)
    cwd: str | None = None             # Optional: working directory
)
```

**Key Points:**
- `env` parameter **overrides specific keys** in the inherited environment (all parent env vars are inherited by default)
- `args` must be a list of strings or None (e.g., `args=["arg1", "arg2"]` or `args=None`)
- All path arguments (`cwd`, values in `args`) should be validated to prevent directory traversal
- Command must be absolute path (e.g., `/bin/bash`) or findable in `PATH`
- The system automatically drops `COLUMNS` and `LINES` from inherited environment to prevent resize issues
- Set `PAR_TERM_REPLY_XTWINOPS=0` to suppress XTWINOPS (CSI t) query responses for security

**Convenience Methods:**
- `term.spawn_shell()` - Auto-detects and spawns default shell (uses `$SHELL` or platform default)
- `PtyTerminal.get_default_shell()` - Returns the default shell path for current platform

## Table of Contents

- [Command Injection Prevention](#command-injection-prevention)
  - [DO: Use Command + Args Array Format](#do-use-command--args-array-format)
  - [DON'T: Concatenate User Input into Commands](#dont-concatenate-user-input-into-commands)
  - [Validating User Input](#validating-user-input)
- [Environment Variable Security](#environment-variable-security)
  - [Inherited Environment](#inherited-environment)
  - [Secure Environment Practices](#secure-environment-practices)
  - [Remove Sensitive Variables](#remove-sensitive-variables)
- [Working Directory Security](#working-directory-security)
  - [Prevent Directory Traversal](#prevent-directory-traversal)
- [Shell Selection Security](#shell-selection-security)
  - [Default Shell Risks](#default-shell-risks)
  - [Secure Shell Usage](#secure-shell-usage)
- [Process Management Security](#process-management-security)
  - [Resource Limits](#resource-limits)
  - [Multiple Session Limits](#multiple-session-limits)
- [Input Validation](#input-validation)
  - [Keyboard Input Sanitization](#keyboard-input-sanitization)
- [Terminal Size Validation](#terminal-size-validation)
- [Privilege Considerations](#privilege-considerations)
  - [Running as Non-Root](#running-as-non-root)
- [Logging and Audit](#logging-and-audit)
  - [Log Security Events](#log-security-events)
- [Summary of Best Practices](#summary-of-best-practices)
- [Security Checklist](#security-checklist)
- [Reporting Security Issues](#reporting-security-issues)

## Command Injection Prevention

### ✅ DO: Use Command + Args Array Format

**SECURE** - Always pass commands and arguments separately:

```python
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)

# GOOD: Command and arguments are separate
term.spawn("/bin/ls", args=["-la", user_input_dir])

# GOOD: Shell with specific command
term.spawn("/bin/sh", args=["-c", safe_command])

# GOOD: With environment and working directory
term.spawn(
    command="/bin/ls",
    args=["-la"],
    env={"SAFE_VAR": "value"},
    cwd="/safe/directory"
)
```

This prevents shell injection because arguments are not interpreted as shell commands.

### ❌ DON'T: Concatenate User Input into Commands

**INSECURE** - Don't build command strings with user input:

```python
# BAD: User input concatenated into shell command
user_file = input("Enter filename: ")  # User enters: file.txt; rm -rf /
dangerous_command = f"cat {user_file}"  # Becomes: cat file.txt; rm -rf /
term.spawn("/bin/sh", args=["-c", dangerous_command])  # EXECUTES rm -rf /!
```

### Validating User Input

If you must use user input in commands, validate and sanitize it:

```python
from pathlib import Path
from par_term_emu_core_rust import PtyTerminal

def safe_spawn_with_file(term, filename):
    """Safely spawn a command with user-provided filename"""
    # Validate filename
    if not filename or '..' in filename or filename.startswith('/'):
        raise ValueError("Invalid filename")

    # Use pathlib for safer path operations
    safe_dir = Path("/safe/directory")
    requested_path = (safe_dir / filename).resolve()

    # Verify it's still in the safe directory (prevents .. traversal)
    try:
        requested_path.relative_to(safe_dir)
    except ValueError:
        raise ValueError("Path traversal attempt detected")

    # Verify file exists and is a regular file
    if not requested_path.is_file():
        raise ValueError("Not a valid file")

    # Use args array format - prevents shell injection
    term.spawn(command="/bin/cat", args=[str(requested_path)])

# Example usage
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
safe_spawn_with_file(term, "document.txt")  # OK: /safe/directory/document.txt
# safe_spawn_with_file(term, "../etc/passwd")  # Raises ValueError
```

## Environment Variable Security

### Inherited Environment

**Important**: `PtyTerminal` inherits ALL parent process environment variables by default, including:
- `PATH` - Can be manipulated to execute malicious binaries
- `LD_PRELOAD` / `LD_LIBRARY_PATH` - Can load malicious libraries
- Authentication tokens and API keys
- Database connection strings

**Automatic Environment Filtering**:
- The system automatically drops `COLUMNS` and `LINES` environment variables (if present in parent) to prevent terminal size conflicts
- These variables are static and don't update on resize, causing issues with terminal-aware applications
- Applications should query terminal size via `ioctl(TIOCGWINSZ)` instead
- Set `PAR_TERM_REPLY_XTWINOPS=0` before creating `PtySession` to suppress XTWINOPS query responses (prevents shell echo visibility with ECHOCTL)

### Secure Environment Practices

```python
from par_term_emu_core_rust import PtyTerminal

# GOOD: Explicitly override sensitive environment variables
# Pass env parameter to override inherited variables with safe values
safe_overrides = {
    "PATH": "/usr/local/bin:/usr/bin:/bin",  # Controlled PATH
    "HOME": "/home/user",
    "USER": "safeuser",
    "TERM": "xterm-256color",
    "COLORTERM": "truecolor",
    # Clear sensitive variables
    "AWS_SECRET_KEY": "",
    "DATABASE_PASSWORD": "",
    "API_TOKEN": "",
}

term = PtyTerminal(80, 24)
term.spawn("/bin/sh", env=safe_overrides)

# Note: env parameter OVERRIDES specified keys in inherited environment
# All other parent environment variables are still inherited
# To completely isolate the environment, you must override ALL inherited vars
```

```python
# CAUTION: This inherits everything from parent
term.spawn_shell()  # Includes all parent env vars (except COLUMNS/LINES)!

# BETTER: Explicitly control environment even with spawn_shell
term.spawn(
    PtyTerminal.get_default_shell(),
    args=None,
    env={
        "PATH": "/usr/local/bin:/usr/bin:/bin",  # Override specific vars
        "AWS_SECRET_KEY": "",                     # Clear sensitive vars
        "DATABASE_PASSWORD": "",
    }
)
```

### Remove Sensitive Variables

```python
import os

# OPTION 1: Remove from parent process environment before spawning
# This affects all future spawn calls
dangerous_vars = ["AWS_SECRET_KEY", "DATABASE_PASSWORD", "API_TOKEN"]

for var in dangerous_vars:
    if var in os.environ:
        del os.environ[var]

term.spawn_shell()
```

```python
# OPTION 2: Override sensitive variables with empty values
# This only affects this specific spawn call
term = PtyTerminal(80, 24)
term.spawn(
    "/usr/bin/app",
    env={
        "AWS_SECRET_KEY": "",     # Override with empty value
        "DATABASE_PASSWORD": "",
        "API_TOKEN": "",
    }
)
```

**Important**: The `env` parameter in `spawn()` **overrides specific keys** in the inherited environment. It does NOT replace the entire environment - all parent environment variables are inherited except those you explicitly override. To create a completely isolated environment, you must explicitly override ALL inherited variables with safe values, or clean the parent's `os.environ` first.

**Security Best Practice**: When using `env` parameter, always explicitly set or clear ALL potentially sensitive environment variables to prevent unintended leakage.

## Working Directory Security

### Prevent Directory Traversal

```python
from pathlib import Path
from par_term_emu_core_rust import PtyTerminal

def safe_spawn_with_cwd(term, user_dir, command, args=None):
    """Safely spawn a process with validated working directory"""
    # Validate and resolve path
    safe_root = Path("/home/user/workspaces")
    requested_path = (safe_root / user_dir).resolve()

    # Verify it's within safe_root (prevents .. traversal)
    try:
        requested_path.relative_to(safe_root)
    except ValueError:
        raise ValueError("Directory traversal attempt detected")

    # Verify directory exists and is a directory
    if not requested_path.is_dir():
        raise ValueError("Not a valid directory")

    # Spawn with validated working directory
    term.spawn(command=command, args=args, cwd=str(requested_path))

# Example usage
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
safe_spawn_with_cwd(term, "project1", "/bin/bash")  # OK
# safe_spawn_with_cwd(term, "../etc", "/bin/bash")  # Raises ValueError
```

## Shell Selection Security

### Default Shell Risks

The `spawn_shell()` method uses the `$SHELL` environment variable:

```python
# Uses $SHELL from environment (could be manipulated!)
term.spawn_shell()
```

**Risk**: If an attacker controls `$SHELL`, they can execute arbitrary programs.

### Secure Shell Usage

```python
import os
from pathlib import Path
from par_term_emu_core_rust import PtyTerminal

# GOOD: Explicitly validate and specify the shell
ALLOWED_SHELLS = ["/bin/bash", "/bin/sh", "/bin/zsh", "/usr/bin/fish"]

def safe_spawn_shell(term):
    """Spawn a shell with validation"""
    # Get shell from environment (could be user-controlled!)
    shell = os.environ.get("SHELL", "/bin/sh")

    # Validate shell is in allowed list
    if shell not in ALLOWED_SHELLS:
        print(f"Warning: Shell {shell} not allowed, using /bin/sh")
        shell = "/bin/sh"

    # Verify shell exists and is executable
    shell_path = Path(shell)
    if not shell_path.is_file() or not os.access(shell, os.X_OK):
        print(f"Warning: Shell {shell} not found or not executable, using /bin/sh")
        shell = "/bin/sh"

    # Spawn the validated shell
    term.spawn(command=shell)

# Example usage
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
safe_spawn_shell(term)
```

**Alternative**: Use the convenience method with default shell detection:
```python
from par_term_emu_core_rust import PtyTerminal

# Uses PtyTerminal.get_default_shell() which defaults to safe values
# On Unix: $SHELL or /bin/bash
# On Windows: PowerShell or cmd.exe
term = PtyTerminal(80, 24)
term.spawn_shell()
```

## Process Management Security

### Resource Limits

Prevent resource exhaustion:

```python
import time
from par_term_emu_core_rust import PtyTerminal

def spawn_with_timeout(term, command, args=None, env=None, cwd=None, timeout=30):
    """Spawn a process with timeout"""
    term.spawn(command=command, args=args, env=env, cwd=cwd)

    start_time = time.time()
    while term.is_running():
        if time.time() - start_time > timeout:
            print(f"Process timeout after {timeout}s")
            term.kill()
            break
        time.sleep(0.1)

    return term.try_wait()

# Example usage
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
exit_code = spawn_with_timeout(term, "/bin/sleep", args=["100"], timeout=5)
print(f"Process exited with code: {exit_code}")  # Will be killed after 5s
```

### Multiple Session Limits

```python
from par_term_emu_core_rust import PtyTerminal

MAX_SESSIONS = 10

class SessionManager:
    def __init__(self):
        self.sessions = []

    def create_session(self, cols, rows):
        # Enforce limit
        if len(self.sessions) >= MAX_SESSIONS:
            raise RuntimeError(f"Maximum {MAX_SESSIONS} sessions allowed")

        term = PtyTerminal(cols, rows)
        self.sessions.append(term)
        return term

    def cleanup(self):
        for term in self.sessions:
            if term.is_running():
                term.kill()
        self.sessions.clear()
```

## Terminal Query Response Security

### XTWINOPS Query Filtering

The terminal emulator automatically responds to certain terminal queries (e.g., XTWINOPS CSI t sequences) to support nested TUI applications. However, when shells have `ECHOCTL` enabled, these responses can become visible in the terminal output, creating visual noise.

**Security Control**: Set the `PAR_TERM_REPLY_XTWINOPS` environment variable to control this behavior:

```python
import os

# Disable XTWINOPS responses (prevents visible query echoes)
os.environ["PAR_TERM_REPLY_XTWINOPS"] = "0"

# Must be set BEFORE creating PtyTerminal
from par_term_emu_core_rust import PtyTerminal
term = PtyTerminal(80, 24)
term.spawn_shell()
```

**Default Behavior**: XTWINOPS responses are **enabled by default** (`PAR_TERM_REPLY_XTWINOPS=1`) to ensure nested TUI applications work correctly. Only disable if you experience visual artifacts or want to prevent potential information leakage via terminal queries.

**When to Disable**:
- Shell environment has `ECHOCTL` enabled (common in some configurations)
- Security-sensitive environments where terminal query responses could leak information
- Testing scenarios where query responses interfere with output validation

**Note**: This setting is cached when `PtySession` is created. Changing the environment variable after session creation has no effect.

## Input Validation

### Keyboard Input Sanitization

```python
from par_term_emu_core_rust import PtyTerminal

def safe_send_input(term, user_input):
    """Safely send user input to terminal"""

    # Limit input length
    if len(user_input) > 4096:
        raise ValueError("Input too long")

    # Check for dangerous control sequences
    dangerous_sequences = [
        b"\x1b[6n",  # Device Status Report (can leak info)
        b"\x1b]",    # OSC sequences (can be misused)
    ]

    input_bytes = user_input.encode('utf-8')
    for seq in dangerous_sequences:
        if seq in input_bytes:
            raise ValueError("Dangerous escape sequence detected")

    term.write(input_bytes)
```

## Terminal Size Validation

```python
from par_term_emu_core_rust import PtyTerminal

def safe_resize(term, cols, rows):
    """Safely resize terminal with validation"""

    # Enforce reasonable limits
    MAX_COLS = 500
    MAX_ROWS = 200
    MIN_COLS = 10
    MIN_ROWS = 5

    if not (MIN_COLS <= cols <= MAX_COLS):
        raise ValueError(f"Columns must be between {MIN_COLS} and {MAX_COLS}")

    if not (MIN_ROWS <= rows <= MAX_ROWS):
        raise ValueError(f"Rows must be between {MIN_ROWS} and {MAX_ROWS}")

    term.resize(cols, rows)
```

## Privilege Considerations

### Running as Non-Root

**Always run PTY applications as non-root users** to limit damage from exploits:

```python
import os
import pwd
from par_term_emu_core_rust import PtyTerminal

def drop_privileges(username="nobody"):
    """Drop privileges to specified user"""
    if os.getuid() != 0:
        # Not root, nothing to drop
        return

    # Get user info
    pwnam = pwd.getpwnam(username)

    # Drop privileges
    os.setgid(pwnam.pw_gid)
    os.setuid(pwnam.pw_uid)

    # Verify we dropped privileges
    if os.getuid() == 0 or os.getgid() == 0:
        raise RuntimeError("Failed to drop privileges")

# Drop privileges before creating PTY sessions
drop_privileges("appuser")

# Now safe to create sessions
term = PtyTerminal(80, 24)
```

## Logging and Audit

### Log Security Events

```python
import logging
from par_term_emu_core_rust import PtyTerminal

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

def secure_spawn(term, command, args=None, env=None, cwd=None, user_id=None):
    """Spawn with audit logging"""

    logger.info(
        f"PTY spawn requested: user={user_id}, "
        f"command={command}, args={args}, "
        f"env_keys={list(env.keys()) if env else None}, "
        f"cwd={cwd}"
    )

    try:
        term.spawn(command=command, args=args, env=env, cwd=cwd)
        logger.info(f"PTY spawn successful: user={user_id}, pid={term.is_running()}")
    except Exception as e:
        logger.error(
            f"PTY spawn failed: user={user_id}, "
            f"error={e}"
        )
        raise

# Example usage
from par_term_emu_core_rust import PtyTerminal

term = PtyTerminal(80, 24)
secure_spawn(
    term,
    "/bin/bash",
    env={"SAFE_MODE": "true"},
    cwd="/home/user/workspace",
    user_id="alice"
)
```

## Summary of Best Practices

1. ✅ **Always use command + args array format** - Never concatenate user input into shell commands
2. ✅ **Validate and sanitize all user input** - Check paths, filenames, and command arguments
3. ✅ **Control environment variables** - Don't blindly inherit parent environment
4. ✅ **Explicitly specify shells** - Don't trust `$SHELL` environment variable
5. ✅ **Implement timeouts** - Kill runaway processes
6. ✅ **Limit resources** - Constrain terminal size, number of sessions, process runtime
7. ✅ **Run as non-root** - Use least-privilege principle
8. ✅ **Log security events** - Audit all PTY operations
9. ✅ **Handle cleanup properly** - Always kill processes when done (use context managers)
10. ✅ **Validate terminal sizes** - Prevent resource exhaustion via extreme dimensions

## Security Checklist

Before deploying PTY functionality:

- [ ] Commands use args array format, not string concatenation
- [ ] All user input is validated and sanitized
- [ ] Sensitive environment variables are removed or overridden
- [ ] Shell is explicitly specified and validated
- [ ] Process timeouts are implemented
- [ ] Resource limits are enforced
- [ ] Application runs as non-root user
- [ ] Security events are logged
- [ ] Cleanup (context managers or explicit kill()) is guaranteed
- [ ] Terminal dimensions are validated
- [ ] Code has been reviewed for injection vulnerabilities
- [ ] Input validation includes escape sequence filtering

## Reporting Security Issues

If you discover a security vulnerability in par-term-emu, please report it to the maintainers privately before public disclosure.
