"""Tests for macro recording and playback functionality."""

import time
import tempfile
from pathlib import Path

import pytest
from par_term_emu_core_rust import Macro, PtyTerminal


def test_macro_creation() -> None:
    """Test creating a macro with friendly key names."""
    macro = Macro("test_macro")
    assert macro.name == "test_macro"
    assert macro.event_count == 0
    assert macro.duration == 0


def test_macro_add_key() -> None:
    """Test adding key events to a macro."""
    macro = Macro("test_keys")
    macro.add_key("ctrl+c")
    macro.add_key("enter")
    macro.add_key("a")

    assert macro.event_count == 3
    events = macro.events
    assert len(events) == 3
    assert events[0].event_type == "key"
    assert events[0].key == "ctrl+c"
    assert events[1].key == "enter"
    assert events[2].key == "a"


def test_macro_add_delay() -> None:
    """Test adding delay events to a macro."""
    macro = Macro("test_delay")
    macro.add_key("a")
    macro.add_delay(100)
    macro.add_key("b")

    assert macro.event_count == 3
    events = macro.events
    assert events[1].event_type == "delay"
    assert events[1].duration == 100


def test_macro_add_screenshot() -> None:
    """Test adding screenshot triggers to a macro."""
    macro = Macro("test_screenshot")
    macro.add_screenshot(None)
    macro.add_screenshot("test.png")

    assert macro.event_count == 2
    events = macro.events
    assert events[0].event_type == "screenshot"
    assert events[0].label is None
    assert events[1].label == "test.png"


def test_macro_yaml_serialization() -> None:
    """Test saving and loading macros to/from YAML."""
    with tempfile.TemporaryDirectory() as tmpdir:
        macro_path = Path(tmpdir) / "test_macro.yaml"

        # Create and save a macro
        macro = Macro("test_yaml")
        macro.set_description("A test macro")
        macro.add_key("ctrl+shift+s")
        macro.add_delay(100)
        macro.add_screenshot("before.png")
        macro.add_key("enter")

        macro.save_yaml(str(macro_path))
        assert macro_path.exists()

        # Load the macro
        loaded = Macro.load_yaml(str(macro_path))
        assert loaded.name == "test_yaml"
        assert loaded.description == "A test macro"
        assert loaded.event_count == 4


def test_macro_to_yaml_string() -> None:
    """Test converting macro to YAML string."""
    macro = Macro("test_yaml_str")
    macro.add_key("a")
    macro.add_delay(50)

    yaml_str = macro.to_yaml()
    assert "name: test_yaml_str" in yaml_str
    assert "type: key" in yaml_str
    assert "type: delay" in yaml_str

    # Test parsing from YAML string
    loaded = Macro.from_yaml(yaml_str)
    assert loaded.name == "test_yaml_str"
    assert loaded.event_count == 2


def test_pty_terminal_macro_library() -> None:
    """Test macro library management in PtyTerminal."""
    term = PtyTerminal(80, 24)

    # Create and load a macro
    macro = Macro("test_lib")
    macro.add_key("ls")
    macro.add_key("enter")

    term.load_macro("my_macro", macro)

    # List macros
    macros = term.list_macros()
    assert "my_macro" in macros

    # Get macro
    retrieved = term.get_macro("my_macro")
    assert retrieved is not None
    assert retrieved.name == "test_lib"

    # Remove macro
    removed = term.remove_macro("my_macro")
    assert removed is not None
    assert removed.name == "test_lib"

    # Verify it's gone
    macros = term.list_macros()
    assert "my_macro" not in macros


def test_macro_playback_simple() -> None:
    """Test basic macro playback."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Create a simple macro
    macro = Macro("test_playback")
    macro.add_key("e")
    macro.add_key("c")
    macro.add_key("h")
    macro.add_key("o")
    macro.add_key("space")
    macro.add_key("h")
    macro.add_key("i")

    # Load and play the macro
    term.load_macro("echo_test", macro)
    term.play_macro("echo_test", speed=100.0)  # Very fast for testing

    # Tick the macro to execute it
    executed = False
    for _ in range(10):
        if term.tick_macro():
            executed = True
        time.sleep(0.01)

    assert executed
    assert not term.is_macro_playing()  # Should be finished


def test_macro_playback_controls() -> None:
    """Test macro playback pause/resume/stop."""
    term = PtyTerminal(80, 24)

    # Create a macro with delays
    macro = Macro("test_controls")
    macro.add_key("a")
    macro.add_delay(1000)
    macro.add_key("b")

    term.load_macro("control_test", macro)
    term.play_macro("control_test", speed=1.0)

    # Check it's playing
    assert term.is_macro_playing()
    assert not term.is_macro_paused()

    # Pause
    term.pause_macro()
    assert term.is_macro_paused()

    # Resume
    term.resume_macro()
    assert not term.is_macro_paused()

    # Stop
    term.stop_macro()
    assert not term.is_macro_playing()


def test_macro_playback_speed() -> None:
    """Test changing macro playback speed."""
    term = PtyTerminal(80, 24)

    macro = Macro("test_speed")
    macro.add_key("a")
    macro.add_delay(100)

    term.load_macro("speed_test", macro)
    term.play_macro("speed_test", speed=0.5)  # Half speed

    assert term.is_macro_playing()

    # Change speed
    term.set_macro_speed(2.0)  # Double speed
    assert term.is_macro_playing()


def test_macro_screenshot_triggers() -> None:
    """Test screenshot triggers during macro playback."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()  # Need to spawn shell before tick_macro

    macro = Macro("test_screenshots")
    macro.add_screenshot("screenshot1.png")
    macro.add_key("a")
    macro.add_screenshot("screenshot2.png")

    term.load_macro("screenshot_test", macro)
    term.play_macro("screenshot_test", speed=100.0)

    # Tick to process events
    for _ in range(5):
        term.tick_macro()

    # Get screenshot triggers
    triggers = term.get_macro_screenshot_triggers()
    assert len(triggers) == 2
    assert "screenshot1.png" in triggers
    assert "screenshot2.png" in triggers

    # Should be empty after retrieval
    triggers = term.get_macro_screenshot_triggers()
    assert len(triggers) == 0


def test_recording_to_macro_conversion() -> None:
    """Test converting a recording session to a macro."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Start recording
    term.start_recording("test recording")

    # Simulate some input
    term.write_str("echo hello")
    time.sleep(0.1)
    term.write_str("\n")
    time.sleep(0.1)

    # Stop recording
    session = term.stop_recording()
    assert session is not None

    # Convert to macro
    macro = term.recording_to_macro(session, "converted_macro")
    assert macro.name == "converted_macro"
    assert macro.event_count > 0

    # Load and verify
    term.load_macro("converted", macro)
    retrieved = term.get_macro("converted")
    assert retrieved is not None


def test_macro_progress_tracking() -> None:
    """Test tracking macro playback progress."""
    term = PtyTerminal(80, 24)

    macro = Macro("test_progress")
    macro.add_key("a")
    macro.add_key("b")
    macro.add_key("c")

    term.load_macro("progress_test", macro)
    term.play_macro("progress_test", speed=100.0)

    # Check initial progress
    progress = term.get_macro_progress()
    assert progress is not None
    current, total = progress
    assert total == 3

    # Get macro name
    name = term.get_current_macro_name()
    assert name == "test_progress"


def test_friendly_key_names() -> None:
    """Test various friendly key name formats."""
    macro = Macro("test_key_names")

    # Control keys
    macro.add_key("ctrl+c")
    macro.add_key("ctrl+shift+s")

    # Function keys
    macro.add_key("f1")
    macro.add_key("f12")

    # Arrow keys
    macro.add_key("up")
    macro.add_key("down")
    macro.add_key("left")
    macro.add_key("right")

    # Special keys
    macro.add_key("enter")
    macro.add_key("tab")
    macro.add_key("backspace")
    macro.add_key("escape")
    macro.add_key("space")

    # Alt keys
    macro.add_key("alt+f4")

    assert (
        macro.event_count == 14
    )  # 2 control + 2 function + 4 arrow + 5 special + 1 alt = 14


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
