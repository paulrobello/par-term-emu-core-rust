"""Extended tests for macro recording and playback functionality.

Comprehensive tests covering edge cases, error handling, performance,
and advanced macro features.
"""

import time

import pytest
from par_term_emu_core_rust import Macro, PtyTerminal


# Edge Cases and Error Handling Tests


def test_macro_empty_creation() -> None:
    """Test creating an empty macro."""
    macro = Macro("empty")
    assert macro.name == "empty"
    assert macro.event_count == 0
    assert macro.duration == 0
    assert macro.events == []


def test_macro_long_name() -> None:
    """Test macro with very long name."""
    long_name = "a" * 1000
    macro = Macro(long_name)
    assert macro.name == long_name


def test_macro_special_characters_in_name() -> None:
    """Test macro with special characters in name."""
    special_names = [
        "macro-with-dashes",
        "macro_with_underscores",
        "macro.with.dots",
        "macro with spaces",
        "macro!@#$%",
    ]

    for name in special_names:
        macro = Macro(name)
        assert macro.name == name


def test_macro_unicode_name() -> None:
    """Test macro with Unicode characters in name."""
    unicode_names = [
        "マクロ",  # Japanese
        "宏",  # Chinese
        "макрос",  # Russian
        "🎮🎯🎨",  # Emojis
    ]

    for name in unicode_names:
        macro = Macro(name)
        assert macro.name == name


def test_macro_very_long_description() -> None:
    """Test macro with very long description."""
    macro = Macro("test")
    long_desc = "x" * 10000
    macro.set_description(long_desc)
    assert macro.description == long_desc


def test_macro_add_many_keys() -> None:
    """Test adding many key events to a macro."""
    macro = Macro("many_keys")

    # Add 1000 key events
    for i in range(1000):
        macro.add_key("a")

    assert macro.event_count == 1000


def test_macro_add_very_long_delay() -> None:
    """Test adding very long delay."""
    macro = Macro("long_delay")
    macro.add_delay(2**32 - 1)  # Max u32 value

    assert macro.event_count == 1
    assert macro.events[0].duration == 2**32 - 1


def test_macro_zero_delay() -> None:
    """Test adding zero-length delay."""
    macro = Macro("zero_delay")
    macro.add_delay(0)

    assert macro.event_count == 1
    assert macro.events[0].duration == 0


def test_macro_screenshot_empty_label() -> None:
    """Test screenshot with empty string label."""
    macro = Macro("screenshot_test")
    macro.add_screenshot("")

    events = macro.events
    assert len(events) == 1
    assert events[0].label == ""


def test_macro_screenshot_very_long_label() -> None:
    """Test screenshot with very long label."""
    macro = Macro("screenshot_test")
    long_label = "x" * 10000
    macro.add_screenshot(long_label)

    events = macro.events
    assert events[0].label == long_label


# YAML Serialization Edge Cases


def test_yaml_serialization_empty_macro() -> None:
    """Test YAML serialization of empty macro."""
    macro = Macro("empty")
    yaml_str = macro.to_yaml()

    assert "name: empty" in yaml_str
    assert "events: []" in yaml_str


def test_yaml_serialization_with_unicode() -> None:
    """Test YAML serialization with Unicode content."""
    macro = Macro("日本語マクロ")
    macro.set_description("Unicode description: 你好世界")
    macro.add_key("a")

    yaml_str = macro.to_yaml()
    loaded = Macro.from_yaml(yaml_str)

    assert loaded.name == macro.name
    assert loaded.description == macro.description


def test_yaml_serialization_preserves_order() -> None:
    """Test that YAML preserves event order."""
    macro = Macro("order_test")

    # Add events in specific order
    for i in range(100):
        macro.add_key(f"key_{i}")

    yaml_str = macro.to_yaml()
    loaded = Macro.from_yaml(yaml_str)

    # Verify order is preserved
    for i, event in enumerate(loaded.events):
        assert event.key == f"key_{i}"


def test_yaml_round_trip_preserves_all_data() -> None:
    """Test that YAML round-trip preserves all macro data."""
    macro = Macro("roundtrip")
    macro.set_description("Test description")
    macro.add_key("ctrl+c")
    macro.add_delay(100)
    macro.add_screenshot("test.png")
    macro.add_key("enter")

    yaml_str = macro.to_yaml()
    loaded = Macro.from_yaml(yaml_str)

    assert loaded.name == macro.name
    assert loaded.description == macro.description
    assert loaded.event_count == macro.event_count
    assert loaded.duration == macro.duration


def test_yaml_load_nonexistent_file() -> None:
    """Test loading from nonexistent file."""
    with pytest.raises(Exception):  # Should raise IOError or similar
        Macro.load_yaml("/nonexistent/path/to/macro.yaml")


def test_yaml_save_invalid_path() -> None:
    """Test saving to invalid path."""
    macro = Macro("test")

    with pytest.raises(Exception):  # Should raise IOError
        macro.save_yaml("/nonexistent/directory/macro.yaml")


def test_yaml_load_invalid_yaml() -> None:
    """Test loading invalid YAML."""
    with pytest.raises(Exception):  # Should raise parse error
        Macro.from_yaml("invalid: yaml: content: [[[")


# Key Parser Tests


def test_key_parser_all_control_keys() -> None:
    """Test parsing all standard control key combinations."""
    control_keys = [
        "ctrl+a",
        "ctrl+b",
        "ctrl+c",
        "ctrl+d",
        "ctrl+z",
    ]

    macro = Macro("control_test")
    for key in control_keys:
        macro.add_key(key)

    assert macro.event_count == len(control_keys)


def test_key_parser_all_function_keys() -> None:
    """Test parsing all function keys."""
    macro = Macro("function_keys")

    for i in range(1, 13):
        macro.add_key(f"f{i}")

    assert macro.event_count == 12


def test_key_parser_case_insensitive() -> None:
    """Test that key parsing is case-insensitive."""
    macro = Macro("case_test")

    # These should all be equivalent
    macro.add_key("CTRL+C")
    macro.add_key("Ctrl+C")
    macro.add_key("ctrl+c")
    macro.add_key("cTrL+c")

    # All should be added (even if they result in same key code)
    assert macro.event_count == 4


def test_key_parser_multiple_modifiers() -> None:
    """Test keys with multiple modifiers."""
    macro = Macro("multi_mod")

    macro.add_key("ctrl+shift+a")
    macro.add_key("ctrl+alt+b")
    macro.add_key("shift+alt+c")
    macro.add_key("ctrl+shift+alt+d")

    assert macro.event_count == 4


def test_key_parser_arrow_keys() -> None:
    """Test all arrow key directions."""
    macro = Macro("arrows")

    for direction in ["up", "down", "left", "right"]:
        macro.add_key(direction)

    assert macro.event_count == 4


def test_key_parser_navigation_keys() -> None:
    """Test navigation keys."""
    nav_keys = ["home", "end", "pageup", "pagedown", "insert", "delete"]

    macro = Macro("nav")
    for key in nav_keys:
        macro.add_key(key)

    assert macro.event_count == len(nav_keys)


def test_key_parser_special_characters() -> None:
    """Test special characters as keys."""
    macro = Macro("special")

    # These should work as single character keys
    for char in "!@#$%^&*()_+-=[]{}\\|;':\"<>?,./":
        macro.add_key(char)

    # Should have added all characters
    assert macro.event_count > 0


# Playback Tests


def test_macro_playback_empty_macro() -> None:
    """Test playing an empty macro."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("empty")
    term.load_macro("empty_test", macro)
    term.play_macro("empty_test", speed=1.0)

    # Should not be playing (empty macro)
    time.sleep(0.1)
    assert not term.is_macro_playing()


def test_macro_playback_nonexistent_macro() -> None:
    """Test playing a macro that doesn't exist."""
    term = PtyTerminal(80, 24)

    with pytest.raises(Exception):  # Should raise ValueError
        term.play_macro("nonexistent", speed=1.0)


def test_macro_playback_very_fast_speed() -> None:
    """Test playback at maximum speed."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("fast")
    for i in range(10):
        macro.add_key("a")
        macro.add_delay(100)

    term.load_macro("fast_test", macro)
    term.play_macro("fast_test", speed=10.0)  # 10x speed

    # Tick through quickly
    start_time = time.time()
    for _ in range(50):
        if not term.is_macro_playing():
            break
        term.tick_macro()
        time.sleep(0.01)

    elapsed = time.time() - start_time

    # Should finish quickly
    assert elapsed < 2.0
    assert not term.is_macro_playing()


def test_macro_playback_very_slow_speed() -> None:
    """Test playback at minimum speed."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("slow")
    macro.add_key("a")
    macro.add_delay(100)

    term.load_macro("slow_test", macro)
    term.play_macro("slow_test", speed=0.1)  # Very slow

    assert term.is_macro_playing()

    # Stop it (would take too long to finish)
    term.stop_macro()
    assert not term.is_macro_playing()


def test_macro_playback_pause_resume_multiple_times() -> None:
    """Test pausing and resuming multiple times."""
    term = PtyTerminal(80, 24)

    macro = Macro("pause_test")
    for i in range(20):
        macro.add_key("a")
        macro.add_delay(50)

    term.load_macro("test", macro)
    term.play_macro("test", speed=1.0)

    # Pause and resume multiple times
    for _ in range(5):
        term.pause_macro()
        assert term.is_macro_paused()

        term.resume_macro()
        assert not term.is_macro_paused()

    term.stop_macro()


def test_macro_playback_change_speed_during_playback() -> None:
    """Test changing speed during playback."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("speed_change")
    for i in range(10):
        macro.add_key("a")
        macro.add_delay(100)

    term.load_macro("test", macro)
    term.play_macro("test", speed=1.0)

    # Change speed multiple times
    speeds = [0.5, 2.0, 1.0, 5.0]
    for speed in speeds:
        term.set_macro_speed(speed)
        time.sleep(0.05)

    # Should still be playing or finished
    term.stop_macro()


def test_macro_playback_stop_while_paused() -> None:
    """Test stopping macro while it's paused."""
    term = PtyTerminal(80, 24)

    macro = Macro("pause_stop")
    macro.add_key("a")
    macro.add_delay(1000)

    term.load_macro("test", macro)
    term.play_macro("test", speed=1.0)

    term.pause_macro()
    assert term.is_macro_paused()

    term.stop_macro()
    assert not term.is_macro_playing()
    assert not term.is_macro_paused()


def test_macro_playback_progress_tracking() -> None:
    """Test detailed progress tracking during playback."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("progress")
    for i in range(10):
        macro.add_key("a")

    term.load_macro("test", macro)
    term.play_macro("test", speed=100.0)

    # Track progress
    progress_points = []
    for _ in range(20):
        if not term.is_macro_playing():
            break

        progress = term.get_macro_progress()
        if progress:
            progress_points.append(progress)

        term.tick_macro()
        time.sleep(0.01)

    # Should have tracked some progress
    assert len(progress_points) > 0

    # Progress should increase
    if len(progress_points) > 1:
        for i in range(len(progress_points) - 1):
            current, _ = progress_points[i]
            next_current, _ = progress_points[i + 1]
            assert next_current >= current


# Library Management Tests


def test_macro_library_add_multiple() -> None:
    """Test adding multiple macros to library."""
    term = PtyTerminal(80, 24)

    # Add many macros
    for i in range(50):
        macro = Macro(f"macro_{i}")
        macro.add_key("a")
        term.load_macro(f"test_{i}", macro)

    # Should have all macros
    macros = term.list_macros()
    assert len(macros) == 50


def test_macro_library_overwrite_existing() -> None:
    """Test overwriting an existing macro in library."""
    term = PtyTerminal(80, 24)

    # Add initial macro
    macro1 = Macro("first")
    macro1.add_key("a")
    term.load_macro("test", macro1)

    # Overwrite with new macro
    macro2 = Macro("second")
    macro2.add_key("b")
    macro2.add_key("c")
    term.load_macro("test", macro2)

    # Should have the new macro
    retrieved = term.get_macro("test")
    assert retrieved.name == "second"
    assert retrieved.event_count == 2


def test_macro_library_remove_nonexistent() -> None:
    """Test removing a macro that doesn't exist."""
    term = PtyTerminal(80, 24)

    removed = term.remove_macro("nonexistent")
    assert removed is None


def test_macro_library_get_nonexistent() -> None:
    """Test getting a macro that doesn't exist."""
    term = PtyTerminal(80, 24)

    retrieved = term.get_macro("nonexistent")
    assert retrieved is None


def test_macro_library_clear_all() -> None:
    """Test clearing all macros from library."""
    term = PtyTerminal(80, 24)

    # Add several macros
    for i in range(10):
        macro = Macro(f"macro_{i}")
        term.load_macro(f"test_{i}", macro)

    # Remove all
    macros = term.list_macros()
    for name in macros:
        term.remove_macro(name)

    # Should be empty
    assert len(term.list_macros()) == 0


# Screenshot Trigger Tests


def test_screenshot_triggers_multiple_in_sequence() -> None:
    """Test multiple screenshot triggers in sequence."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("screenshots")
    for i in range(10):
        macro.add_screenshot(f"screen_{i}.png")
        macro.add_delay(10)

    term.load_macro("test", macro)
    term.play_macro("test", speed=100.0)

    # Collect all triggers
    all_triggers = []
    for _ in range(30):
        term.tick_macro()
        triggers = term.get_macro_screenshot_triggers()
        all_triggers.extend(triggers)
        time.sleep(0.01)

    # Should have collected all 10 triggers
    assert len(all_triggers) == 10


def test_screenshot_triggers_cleared_after_retrieval() -> None:
    """Test that screenshot triggers are cleared after being retrieved."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("screenshot_clear")
    macro.add_screenshot("test1.png")
    macro.add_screenshot("test2.png")

    term.load_macro("test", macro)
    term.play_macro("test", speed=100.0)

    # Process events
    for _ in range(5):
        term.tick_macro()

    # Get triggers
    triggers = term.get_macro_screenshot_triggers()
    assert len(triggers) > 0

    # Get again - should be empty
    triggers2 = term.get_macro_screenshot_triggers()
    assert len(triggers2) == 0


# Recording Conversion Tests


def test_recording_to_macro_preserves_timing() -> None:
    """Test that converting recording to macro preserves timing."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Start recording
    term.start_recording("timing_test")

    # Write with delays
    term.write_str("a")
    time.sleep(0.1)
    term.write_str("b")
    time.sleep(0.1)
    term.write_str("c")

    time.sleep(0.1)
    session = term.stop_recording()

    # Convert to macro
    macro = term.recording_to_macro(session, "timing_macro")

    # Should have delays between keys
    has_delays = any(event.event_type == "delay" for event in macro.events)
    assert has_delays


def test_recording_to_macro_empty_recording() -> None:
    """Test converting empty recording to macro."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    term.start_recording("empty")
    session = term.stop_recording()

    macro = term.recording_to_macro(session, "empty_macro")

    # Should create a macro even if empty
    assert macro.name == "empty_macro"


# Complex Macro Scenarios


def test_macro_complex_sequence() -> None:
    """Test a complex macro with mixed event types."""
    macro = Macro("complex")

    # Build a complex sequence
    macro.add_key("ctrl+c")
    macro.add_delay(100)
    macro.add_screenshot("after_ctrl_c.png")
    macro.add_key("l")
    macro.add_key("s")
    macro.add_key("space")
    macro.add_key("-")
    macro.add_key("l")
    macro.add_key("a")
    macro.add_delay(50)
    macro.add_screenshot("after_ls.png")
    macro.add_key("enter")
    macro.add_delay(200)
    macro.add_screenshot("final.png")

    assert macro.event_count == 14
    assert macro.duration == 350


def test_macro_chained_execution() -> None:
    """Test executing multiple macros in sequence."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    # Create three macros
    macro1 = Macro("first")
    macro1.add_key("a")

    macro2 = Macro("second")
    macro2.add_key("b")

    macro3 = Macro("third")
    macro3.add_key("c")

    # Load all
    term.load_macro("m1", macro1)
    term.load_macro("m2", macro2)
    term.load_macro("m3", macro3)

    # Execute in sequence
    for name in ["m1", "m2", "m3"]:
        term.play_macro(name, speed=100.0)

        # Wait for completion
        while term.is_macro_playing():
            term.tick_macro()
            time.sleep(0.01)


@pytest.mark.slow
@pytest.mark.timeout(30)
def test_macro_very_long_playback() -> None:
    """Test playing back a very long macro."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()

    macro = Macro("long")

    # Create a long sequence (500 events)
    for i in range(500):
        macro.add_key("a")
        if i % 10 == 0:
            macro.add_delay(10)

    term.load_macro("long_test", macro)
    term.play_macro("long_test", speed=10.0)

    # Play through (with timeout)
    max_iterations = 1000
    iterations = 0

    while term.is_macro_playing() and iterations < max_iterations:
        term.tick_macro()
        iterations += 1
        time.sleep(0.001)

    # Should have completed or hit timeout
    assert iterations < max_iterations or not term.is_macro_playing()


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-m", "not slow"])
