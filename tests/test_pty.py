"""
Integration tests for PTY functionality
"""

import sys

import pytest
from conftest import wait_for


def test_import_pty_terminal():
    """Test that PtyTerminal can be imported"""
    from par_term_emu_core_rust import PtyTerminal

    assert PtyTerminal is not None


def test_create_pty_terminal():
    """Test creating a PtyTerminal instance"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    assert term is not None
    assert term.size() == (80, 24)
    assert not term.is_running()


def test_create_pty_terminal_with_scrollback():
    """Test creating a PtyTerminal with custom scrollback"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24, scrollback=5000)
    assert term.size() == (80, 24)


def test_invalid_dimensions():
    """Test that zero dimensions raise an error"""
    from par_term_emu_core_rust import PtyTerminal

    with pytest.raises(ValueError):
        PtyTerminal(0, 24)

    with pytest.raises(ValueError):
        PtyTerminal(80, 0)


def test_get_default_shell():
    """Test getting the default shell"""
    from par_term_emu_core_rust import PtyTerminal

    shell = PtyTerminal.get_default_shell()
    assert isinstance(shell, str)
    assert len(shell) > 0


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_simple_command_unix():
    """Test spawning a simple command that exits immediately (Unix)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/echo", args=["hello", "world"])

    # Check that content was captured
    assert wait_for(lambda: "hello" in term.content() or "world" in term.content())

    # Process should have exited
    exit_code = term.try_wait()
    assert exit_code is not None
    assert exit_code == 0


@pytest.mark.skipif(sys.platform != "win32", reason="Windows-specific test")
def test_spawn_simple_command_windows():
    """Test spawning a simple command that exits immediately (Windows)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("cmd.exe", args=["/C", "echo hello world"])

    # Check that content was captured
    assert wait_for(lambda: "hello" in term.content())

    # Process should have exited
    exit_code = term.try_wait()
    assert exit_code is not None
    assert exit_code == 0


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_write_to_process_unix():
    """Test writing to a running process (Unix)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/cat")

    assert term.is_running()

    # Write to cat
    term.write_str("hello\n")

    # cat should echo it back
    assert wait_for(lambda: "hello" in term.content())

    # Kill the process
    term.kill()
    assert wait_for(lambda: not term.is_running())


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_with_env_vars():
    """Test spawning with custom environment variables"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn(
        "/bin/sh",
        args=["-c", "echo $TEST_VAR"],
        env={"TEST_VAR": "test_value"},
    )

    assert wait_for(lambda: "test_value" in term.content())


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_with_cwd():
    """Test spawning with custom working directory"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/pwd", cwd="/tmp")

    assert wait_for(lambda: "/tmp" in term.content())


def test_resize():
    """Test resizing the PTY"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    assert term.size() == (80, 24)

    term.resize(100, 30)
    assert term.size() == (100, 30)


def test_resize_invalid():
    """Test that zero dimensions in resize raise an error"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)

    with pytest.raises(ValueError):
        term.resize(0, 24)

    with pytest.raises(ValueError):
        term.resize(80, 0)


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_wait_for_process():
    """Test waiting for a process to exit"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/sh", args=["-c", "exit 42"])

    # Wait for process to exit
    exit_code = term.wait()
    assert exit_code == 42
    assert not term.is_running()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_kill_process():
    """Test killing a running process"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/sleep", args=["10"])

    assert term.is_running()

    term.kill()
    assert wait_for(lambda: not term.is_running())


def test_terminal_query_methods():
    """Test terminal query methods"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)

    # Test basic queries without spawning
    assert term.size() == (80, 24)
    assert term.cursor_position() == (0, 0)
    # Empty terminal contains spaces/newlines (initialized cells)
    assert len(term.content()) > 0  # Has content (spaces)
    assert term.content().strip() == ""  # But all whitespace
    assert term.scrollback() == []
    assert term.scrollback_len() == 0


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_get_line():
    """Test getting specific lines from the terminal"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/echo", args=["test"])

    assert wait_for(lambda: "test" in (term.get_line(0) or ""))

    # Get the first line
    line = term.get_line(0)
    assert line is not None
    assert "test" in line or line == ""


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_get_char_and_colors():
    """Test getting character and color information"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/echo", args=["test"])

    assert wait_for(lambda: "test" in term.content())

    # Try to get a character (might be None if position is empty)
    term.get_char(0, 0)
    # Character might be None or a character

    # Get colors (should work even if empty)
    term.get_fg_color(0, 0)
    term.get_bg_color(0, 0)

    # Get attributes
    term.get_attributes(0, 0)


def test_pty_terminal_title():
    """Test getting terminal title from PtyTerminal"""
    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference)
    term_regular = Terminal(80, 24)
    term_regular.process_str("\x1b]0;Regular Title\x07")
    assert term_regular.title() == "Regular Title"

    # Now test PtyTerminal has the same API
    pty_term = PtyTerminal(80, 24)

    # Check initial title is empty
    assert hasattr(pty_term, "title"), "PtyTerminal should have title() method"
    assert pty_term.title() == ""

    # Note: Since PTY tests are skipped in CI and we can't actually send
    # sequences through a running PTY process here, we've verified:
    # 1. The method exists
    # 2. It returns a string (empty initially)
    # 3. It matches the Terminal API contract


def test_update_generation_initial():
    """Test that generation counter starts at 0"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    assert term.update_generation() == 0


def test_has_updates_since_no_changes():
    """Test that has_updates_since returns False when no changes occurred"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    gen = term.update_generation()
    assert not term.has_updates_since(gen)


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_generation_counter_increments_on_output():
    """Regression test for issue #60: generation counter must increment on PTY output."""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/echo", args=["hello"])

    gen_before = term.update_generation()
    assert wait_for(lambda: term.update_generation() > gen_before)

    gen_after = term.update_generation()
    assert gen_after > gen_before, (
        f"generation counter should increment after PTY output: was {gen_before}, now {gen_after}"
    )
    assert term.has_updates_since(gen_before), (
        "has_updates_since() should return True after PTY output"
    )


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_generation_counter_after_ctrl_c():
    """Regression test for issue #60: generation counter must still work after Ctrl+C."""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn_shell()
    assert wait_for(lambda: term.content().strip())

    # Send Ctrl+C
    term.write(b"\x03")

    # Send a normal command after Ctrl+C
    gen_before = term.update_generation()
    term.write_str("echo GENERATION_TEST\n")
    assert wait_for(lambda: term.update_generation() > gen_before)

    gen_after = term.update_generation()
    assert gen_after > gen_before, (
        f"generation counter should increment for output after Ctrl+C: was {gen_before}, now {gen_after}"
    )
    assert term.has_updates_since(gen_before), (
        "has_updates_since() must detect changes after Ctrl+C"
    )


def test_pty_terminal_hyperlink():
    """Test getting hyperlink from PtyTerminal"""
    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference)
    term_regular = Terminal(80, 24)
    term_regular.process_str("\x1b]8;;https://example.com\x07Click\x1b]8;;\x07")
    assert term_regular.get_hyperlink(0, 0) == "https://example.com"

    # Verify PtyTerminal has the same API
    pty_term = PtyTerminal(80, 24)
    assert hasattr(pty_term, "get_hyperlink"), (
        "PtyTerminal should have get_hyperlink() method"
    )
    assert pty_term.get_hyperlink(0, 0) is None  # Empty initially

    # Note: Since PTY tests are skipped in CI and we can't actually send
    # sequences through a running PTY process here, we've verified:
    # 1. The method exists
    # 2. It returns None for positions without hyperlinks
    # 3. It matches the Terminal API contract


def test_pty_terminal_flush_synchronized_updates():
    """Test flush_synchronized_updates from PtyTerminal"""
    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference)
    term_regular = Terminal(80, 24)
    term_regular.flush_synchronized_updates()  # Should not raise

    # Verify PtyTerminal has the same API
    pty_term = PtyTerminal(80, 24)
    assert hasattr(pty_term, "flush_synchronized_updates"), (
        "PtyTerminal should have flush_synchronized_updates() method"
    )
    pty_term.flush_synchronized_updates()  # Should not raise

    # Note: Since PTY tests are skipped in CI, we've verified:
    # 1. The method exists
    # 2. It can be called without errors
    # 3. It matches the Terminal API contract


def test_pty_terminal_focus_events():
    """Test focus event methods from PtyTerminal"""
    import sys

    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference). Focus reporting
    # follows xterm DEC 1004 semantics: no events until the application
    # opts in with CSI ? 1004 h.
    term_regular = Terminal(80, 24)
    assert term_regular.get_focus_in_event() == b""
    assert term_regular.get_focus_out_event() == b""

    term_regular.process_str("\x1b[?1004h")
    assert term_regular.get_focus_in_event() == b"\x1b[I"
    assert term_regular.get_focus_out_event() == b"\x1b[O"

    # PtyTerminal: the mode can only be enabled by the child writing the
    # sequence through the PTY (there is no process_str on PtyTerminal).
    if sys.platform != "win32":
        pty_term = PtyTerminal(80, 24)
        assert pty_term.get_focus_in_event() == b""
        pty_term.spawn("/bin/sh", args=["-c", "printf '\x1b[?1004h'"])
        assert wait_for(lambda: pty_term.get_focus_in_event() == b"\x1b[I")
        assert pty_term.get_focus_out_event() == b"\x1b[O"


def test_context_manager():
    """Test using PtyTerminal as a context manager"""
    from par_term_emu_core_rust import PtyTerminal

    with PtyTerminal(80, 24) as term:
        assert term.size() == (80, 24)

    # After context exit, cleanup should have happened


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_context_manager_with_process():
    """Test context manager cleanup with running process"""
    from par_term_emu_core_rust import PtyTerminal

    with PtyTerminal(80, 24) as term:
        term.spawn("/bin/sleep", args=["10"])
        assert term.is_running()

    # Process should be killed after context exit
    # (we can't check term.is_running() here as term is out of scope)


def test_repr_and_str():
    """Test __repr__ and __str__ methods"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)

    repr_str = repr(term)
    assert "PtyTerminal" in repr_str
    assert "80" in repr_str
    assert "24" in repr_str

    str_str = str(term)
    assert isinstance(str_str, str)


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_multiple_writes():
    """Test multiple writes to a process"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/cat")

    # Writes to the same PTY are ordered, so no pacing between them is needed.
    term.write_str("line1\n")
    term.write_str("line2\n")
    term.write_str("line3\n")

    # All three lines should be echoed back
    assert wait_for(lambda: all(f"line{i}" in term.content() for i in (1, 2, 3)))

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_shell():
    """Test spawning the default shell"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn_shell()

    assert term.is_running()

    # Write a simple command
    term.write_str("echo test\n")
    assert wait_for(lambda: "test" in term.content())

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_write_without_spawn_fails():
    """Test that writing without spawning a process fails"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)

    # Should raise an error since no process is running
    with pytest.raises(RuntimeError):
        term.write_str("test")


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_shell_with_env():
    """Test spawning shell with environment variables passed directly"""
    import os

    from par_term_emu_core_rust import PtyTerminal

    # Use a unique var name to avoid conflicts
    unique_var = "PTY_TEST_SHELL_ENV_VAR_12345"

    # Verify it doesn't exist in current process
    assert unique_var not in os.environ, "Test var should not exist before spawn"

    term = PtyTerminal(80, 24)
    term.spawn_shell(env={unique_var: "hello_from_shell"})

    assert term.is_running()

    # Echo the var to verify it was passed
    term.write_str(f"echo ${unique_var}\n")
    assert wait_for(lambda: "hello_from_shell" in term.content()), (
        f"Expected env var value in output, got: {term.content()}"
    )

    # Verify the var was NOT leaked to parent process
    assert unique_var not in os.environ, (
        "Test var should NOT exist in parent after spawn"
    )

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_shell_with_cwd():
    """Test spawning shell with working directory set"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn_shell(cwd="/tmp")

    assert term.is_running()

    # Print current directory
    term.write_str("pwd\n")
    assert wait_for(
        lambda: "/tmp" in term.content() or "/private/tmp" in term.content()
    ), f"Expected /tmp in output, got: {term.content()}"

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_shell_backward_compatible():
    """Test that spawn_shell() with no args still works (backward compatibility)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    # Call without any arguments - should work like before
    term.spawn_shell()

    assert term.is_running()

    term.write_str("echo backward_compat_test\n")
    assert wait_for(lambda: "backward_compat_test" in term.content())

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_shell_with_env_and_cwd():
    """Test spawning shell with both env and cwd"""
    import os

    from par_term_emu_core_rust import PtyTerminal

    unique_var = "PTY_TEST_COMBINED_VAR_67890"
    assert unique_var not in os.environ

    term = PtyTerminal(80, 24)
    term.spawn_shell(env={unique_var: "combined_test"}, cwd="/tmp")

    assert term.is_running()

    # Verify both env var and cwd
    term.write_str(f"echo ${unique_var} && pwd\n")

    def _combined_output() -> str:
        content = term.content()
        return content if ("combined_test" in content and "/tmp" in content) else ""

    assert wait_for(_combined_output), (
        f"Expected env var value and /tmp in output, got: {term.content()}"
    )

    # Verify parent env unchanged
    assert unique_var not in os.environ

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_with_env_dict():
    """Test spawn() with env dict parameter"""
    import os

    from par_term_emu_core_rust import PtyTerminal

    unique_var = "PTY_TEST_SPAWN_ENV_VAR"
    assert unique_var not in os.environ

    term = PtyTerminal(80, 24)
    # spawn() already supports env parameter
    term.spawn(
        "/bin/sh", ["-c", f"echo ${unique_var}"], env={unique_var: "spawn_env_value"}
    )

    assert wait_for(lambda: "spawn_env_value" in term.content()), (
        f"Expected env var value in output, got: {term.content()}"
    )

    # Verify parent env unchanged
    assert unique_var not in os.environ


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_wait_for_update_advances_on_output():
    """wait_for_update blocks until the reader applies new output (ENH-011)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    gen = term.update_generation()
    term.spawn("/bin/echo", args=["wait-marker"])
    assert term.wait_for_update(gen, timeout=3.0) is not None


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_wait_for_text_finds_and_times_out():
    """wait_for_text wakes on the content and honors the timeout (ENH-011)"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/echo", args=["wait-text-marker"])
    assert term.wait_for_text("wait-text-marker", timeout=3.0)

    # A LIVE child that stays silent: the timeout path takes the full
    # window. (An exited child returns early by design — the child-gone
    # fast path skips waiting out the deadline.)
    import time as _time

    term2 = PtyTerminal(80, 24)
    term2.spawn("/bin/cat", [])
    start = _time.monotonic()
    assert not term2.wait_for_text("never-appears", timeout=0.3)
    assert 0.25 <= _time.monotonic() - start < 2.0


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_wait_for_text_releases_the_gil():
    """A blocking wait must not starve other Python threads (ENH-011).

    The wait times out on absent text, so the whole window is spent blocked:
    if the GIL were held, the spinner thread would be starved for the full
    0.5 s instead of ticking every ~1 ms.
    """
    import threading
    import time as _time

    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/cat", [])
    ticks = {"n": 0}
    stop = threading.Event()

    def spin():
        while not stop.is_set():
            ticks["n"] += 1
            _time.sleep(0.001)

    spinner = threading.Thread(target=spin)
    spinner.start()
    try:
        start = _time.monotonic()
        assert not term.wait_for_text("never-appears", timeout=0.5)
        assert _time.monotonic() - start >= 0.45
    finally:
        stop.set()
        spinner.join()

    assert ticks["n"] > 50, (
        f"wait_for_text starved a concurrent thread ({ticks['n']} ticks in 0.5 s); "
        "the GIL must be released while blocking"
    )


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
