"""
Integration tests for PTY functionality
"""

import time
import sys
import pytest

pytestmark = pytest.mark.skip(reason="PTY tests hang in CI")


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

    # Give it time to execute and capture output
    time.sleep(0.2)

    # Check that content was captured
    content = term.content()
    assert "hello" in content or "world" in content

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

    # Give it time to execute
    time.sleep(0.2)

    # Check that content was captured
    content = term.content()
    assert "hello" in content

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
    time.sleep(0.1)

    # cat should echo it back
    content = term.content()
    assert "hello" in content

    # Kill the process
    term.kill()
    time.sleep(0.1)
    assert not term.is_running()


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

    time.sleep(0.2)

    content = term.content()
    assert "test_value" in content


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_spawn_with_cwd():
    """Test spawning with custom working directory"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)
    term.spawn("/bin/pwd", cwd="/tmp")

    time.sleep(0.2)

    content = term.content()
    assert "/tmp" in content


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
    time.sleep(0.1)

    assert not term.is_running()


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

    time.sleep(0.2)

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

    time.sleep(0.2)

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
    from par_term_emu_core_rust import PtyTerminal, Terminal

    # Test with regular Terminal first (as reference)
    term_regular = Terminal(80, 24)
    focus_in = term_regular.get_focus_in_event()
    focus_out = term_regular.get_focus_out_event()
    assert focus_in == b"\x1b[I"
    assert focus_out == b"\x1b[O"

    # Verify PtyTerminal has the same API
    pty_term = PtyTerminal(80, 24)
    assert hasattr(pty_term, "get_focus_in_event"), (
        "PtyTerminal should have get_focus_in_event() method"
    )
    assert hasattr(pty_term, "get_focus_out_event"), (
        "PtyTerminal should have get_focus_out_event() method"
    )

    pty_focus_in = pty_term.get_focus_in_event()
    pty_focus_out = pty_term.get_focus_out_event()
    assert pty_focus_in == b"\x1b[I"
    assert pty_focus_out == b"\x1b[O"

    # Note: Since PTY tests are skipped in CI, we've verified:
    # 1. Both methods exist
    # 2. They return the correct event sequences
    # 3. They match the Terminal API contract


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

    # Write multiple times
    term.write_str("line1\n")
    time.sleep(0.05)
    term.write_str("line2\n")
    time.sleep(0.05)
    term.write_str("line3\n")
    time.sleep(0.1)

    content = term.content()
    # At least one line should be captured
    assert "line" in content

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
    time.sleep(0.2)

    content = term.content()
    # Shell should produce some output
    assert len(content) > 0

    term.kill()


@pytest.mark.skipif(sys.platform == "win32", reason="Unix-specific test")
def test_write_without_spawn_fails():
    """Test that writing without spawning a process fails"""
    from par_term_emu_core_rust import PtyTerminal

    term = PtyTerminal(80, 24)

    # Should raise an error since no process is running
    with pytest.raises(RuntimeError):
        term.write_str("test")


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
