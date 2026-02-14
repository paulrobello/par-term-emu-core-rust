"""Tests for command output capture (issue #36)."""

from par_term_emu_core_rust import Terminal


class TestGetCommandOutput:
    """Test get_command_output() method."""

    def test_basic_output_capture(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"file1.txt\r\nfile2.txt\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        output = term.get_command_output(0)
        assert output is not None
        assert "file1.txt" in output
        assert "file2.txt" in output

    def test_returns_none_for_empty_history(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_command_output(0) is None

    def test_returns_none_for_out_of_bounds(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)
        assert term.get_command_output(5) is None

    def test_command_without_zones_returns_none(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("echo hi")
        term.end_command_execution(0)
        assert term.get_command_output(0) is None

    def test_multiple_commands_indexed_correctly(self) -> None:
        term = Terminal(80, 24, scrollback=100)

        # First command
        term.start_command_execution("cmd1")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07cmd1\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output1\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        # Second command
        term.start_command_execution("cmd2")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07cmd2\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output2\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        # 0 = most recent
        out0 = term.get_command_output(0)
        assert out0 is not None
        assert "output2" in out0

        out1 = term.get_command_output(1)
        assert out1 is not None
        assert "output1" in out1

    def test_empty_output_returns_string(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("true")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        output = term.get_command_output(0)
        # Output zone exists but may have no content
        assert output is not None


class TestGetCommandOutputs:
    """Test get_command_outputs() bulk method."""

    def test_empty_history(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_command_outputs() == []

    def test_returns_dict_with_expected_keys(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"hello\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        outputs = term.get_command_outputs()
        assert len(outputs) == 1
        assert "command" in outputs[0]
        assert "cwd" in outputs[0]
        assert "exit_code" in outputs[0]
        assert "output" in outputs[0]
        assert outputs[0]["command"] == "ls"
        assert "hello" in outputs[0]["output"]

    def test_excludes_commands_without_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # Command without zones
        term.start_command_execution("no-zones")
        term.end_command_execution(0)

        # Command with zones
        term.start_command_execution("with-zones")
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        outputs = term.get_command_outputs()
        commands = [o["command"] for o in outputs]
        assert "no-zones" not in commands
        assert "with-zones" in commands


class TestCommandExecutionFields:
    """Test that CommandExecution has the new output row fields."""

    def test_output_rows_on_command_execution(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("ls")
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output\r\n")
        term.process(b"\x1b]133;D;0\x07")
        term.end_command_execution(0)

        history = term.get_command_history()
        assert len(history) == 1
        assert history[0].output_start_row is not None
        assert history[0].output_end_row is not None
        assert history[0].output_start_row <= history[0].output_end_row

    def test_no_output_rows_without_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.start_command_execution("echo")
        term.end_command_execution(0)

        history = term.get_command_history()
        assert len(history) == 1
        assert history[0].output_start_row is None
        assert history[0].output_end_row is None
