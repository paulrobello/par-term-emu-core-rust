"""Tests for semantic buffer zoning (OSC 133 shell integration zones)."""

from par_term_emu_core_rust import Terminal


class TestZoneCreation:
    """Test zone creation via OSC 133 markers."""

    def test_no_zones_initially(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zones() == []

    def test_prompt_zone_created_on_osc_133_a(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert len(zones) == 1
        assert zones[0]["zone_type"] == "prompt"

    def test_full_command_cycle_creates_three_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # Prompt
        term.process(b"\x1b]133;A\x07")
        # Command
        term.process(b"\x1b]133;B\x07")
        # Output
        term.process(b"\x1b]133;C\x07")
        term.process(b"hello\r\n")
        # Finished
        term.process(b"\x1b]133;D;0\x07")

        zones = term.get_zones()
        assert len(zones) == 3
        assert zones[0]["zone_type"] == "prompt"
        assert zones[1]["zone_type"] == "command"
        assert zones[2]["zone_type"] == "output"
        assert zones[2]["exit_code"] == 0

    def test_exit_code_nonzero(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"\x1b]133;D;127\x07")

        zones = term.get_zones()
        assert zones[2]["exit_code"] == 127

    def test_zone_timestamps_are_set(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert zones[0]["timestamp"] is not None
        assert zones[0]["timestamp"] > 0

    def test_multiple_command_cycles(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        for _ in range(3):
            term.process(b"\x1b]133;A\x07")
            term.process(b"\x1b]133;B\x07")
            term.process(b"\x1b]133;C\x07")
            term.process(b"\x1b]133;D;0\x07")

        zones = term.get_zones()
        assert len(zones) == 9  # 3 cycles * 3 zones each


class TestZoneQuery:
    """Test zone query methods."""

    def test_get_zone_at_returns_correct_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # Put each marker on separate lines so zones don't overlap
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output line\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Row 0 should be in the Prompt zone
        zone = term.get_zone_at(0)
        assert zone is not None
        assert zone["zone_type"] == "prompt"

    def test_get_zone_at_returns_none_for_no_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zone_at(0) is None

    def test_get_zone_at_returns_none_beyond_zones(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        # Query a row far beyond any zone
        assert term.get_zone_at(1000) is None

    def test_get_zone_at_command_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output line\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Row 1 should be in the Command zone
        zone = term.get_zone_at(1)
        assert zone is not None
        assert zone["zone_type"] == "command"

    def test_get_zone_at_output_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output line\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Row 2 should be in the Output zone
        zone = term.get_zone_at(2)
        assert zone is not None
        assert zone["zone_type"] == "output"


class TestZoneText:
    """Test zone text extraction."""

    def test_get_zone_text_extracts_content(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")
        term.process(b"file1.txt\r\nfile2.txt\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Get text of the prompt zone (first zone, row 0)
        text = term.get_zone_text(0)
        assert text is not None
        assert "$" in text

    def test_get_zone_text_returns_none_for_no_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        assert term.get_zone_text(0) is None

    def test_get_zone_text_output_zone(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"file1.txt\r\nfile2.txt\r\n")
        term.process(b"\x1b]133;D;0\x07")

        # Get text of the output zone (row 2 is in the output zone)
        text = term.get_zone_text(2)
        assert text is not None
        assert "file1.txt" in text
        assert "file2.txt" in text


class TestZoneEviction:
    """Test zone eviction on scrollback wrap."""

    def test_zones_evicted_when_scrollback_wraps(self) -> None:
        term = Terminal(80, 5, scrollback=10)

        # Create initial zones
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")

        # Generate enough output to fill scrollback and trigger eviction
        for i in range(25):
            term.process(f"line {i}\r\n".encode())
        term.process(b"\x1b]133;D;0\x07")

        # Early zones should have been evicted or truncated
        zones = term.get_zones()
        # Zones that remain should have row ends within the valid range
        for z in zones:
            assert z["abs_row_end"] >= 0  # Basic sanity
            assert z["abs_row_start"] <= z["abs_row_end"]


class TestZoneAltScreen:
    """Test zone behavior with alternate screen."""

    def test_no_zones_on_alt_screen(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        # Switch to alt screen
        term.process(b"\x1b[?1049h")
        term.process(b"\x1b]133;A\x07")
        assert term.get_zones() == []

        # Switch back
        term.process(b"\x1b[?1049l")
        term.process(b"\x1b]133;A\x07")
        assert len(term.get_zones()) == 1


class TestZoneReset:
    """Test zones cleared on terminal reset."""

    def test_zones_cleared_on_reset(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        assert len(term.get_zones()) == 1

        # Full reset (RIS)
        term.process(b"\x1bc")
        assert term.get_zones() == []


class TestZoneDictFields:
    """Test that zone dictionaries have all expected fields."""

    def test_zone_dict_has_all_fields(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert len(zones) == 1
        zone = zones[0]
        assert "zone_type" in zone
        assert "abs_row_start" in zone
        assert "abs_row_end" in zone
        assert "command" in zone
        assert "exit_code" in zone
        assert "timestamp" in zone

    def test_prompt_zone_has_no_command(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert zones[0]["command"] is None

    def test_prompt_zone_has_no_exit_code(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        zones = term.get_zones()
        assert zones[0]["exit_code"] is None

    def test_output_zone_exit_code_set_on_finish(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07")
        term.process(b"\x1b]133;B\x07")
        term.process(b"\x1b]133;C\x07")

        # Before D, the output zone has no exit code
        zones = term.get_zones()
        assert zones[2]["exit_code"] is None

        # After D;42, exit code should be set
        term.process(b"\x1b]133;D;42\x07")
        zones = term.get_zones()
        assert zones[2]["exit_code"] == 42

    def test_zone_abs_row_start_and_end(self) -> None:
        term = Terminal(80, 24, scrollback=100)
        term.process(b"\x1b]133;A\x07$ \r\n")
        term.process(b"\x1b]133;B\x07ls\r\n")
        term.process(b"\x1b]133;C\x07")
        term.process(b"output\r\n")
        term.process(b"\x1b]133;D;0\x07")

        zones = term.get_zones()
        # Prompt zone: starts at row 0, closed when B arrives on row 1
        assert zones[0]["abs_row_start"] == 0
        assert zones[0]["abs_row_end"] == 0

        # Command zone: starts at row 1, closed when C arrives on row 2
        assert zones[1]["abs_row_start"] == 1
        assert zones[1]["abs_row_end"] == 1

        # Output zone: starts at row 2, closed when D arrives
        assert zones[2]["abs_row_start"] == 2
