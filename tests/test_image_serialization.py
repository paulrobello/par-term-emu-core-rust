"""Tests for image metadata serialization for session persistence."""

import json
import tempfile
from pathlib import Path

from par_term_emu_core_rust import Terminal


def _create_sixel_graphic(term: Terminal) -> None:
    """Helper to add a sixel graphic to the terminal."""
    sixel = (
        "\x1bPq"  # DCS Sixel start
        '"1;1;2;6'  # raster: pan=1, pad=1, width=2, height=6
        "#0;2;100;0;0"  # define color 0 = red
        "#0"  # select color 0
        "~~"  # two columns filled
        "\x1b\\"  # ST
    )
    term.process_str(sixel)


def test_export_graphics_json_empty():
    """Exporting with no graphics returns valid JSON with empty arrays."""
    term = Terminal(80, 24)
    json_str = term.export_graphics_json()
    data = json.loads(json_str)
    assert data["version"] == 1
    assert data["placements"] == []
    assert data["scrollback"] == []
    assert data["animations"] == []


def test_export_graphics_json_with_sixel():
    """Exporting after adding a sixel graphic produces valid JSON."""
    term = Terminal(80, 24)
    _create_sixel_graphic(term)
    assert term.graphics_count() >= 1

    json_str = term.export_graphics_json()
    data = json.loads(json_str)

    assert data["version"] == 1
    assert len(data["placements"]) >= 1

    placement = data["placements"][0]
    assert placement["protocol"] == "Sixel"
    assert "data" in placement
    assert placement["data"]["type"] == "Inline"
    assert isinstance(placement["data"]["value"], str)  # base64 string
    assert placement["width"] > 0
    assert placement["height"] > 0


def test_import_graphics_json_round_trip():
    """Graphics exported from one terminal can be imported into another."""
    term1 = Terminal(80, 24)
    _create_sixel_graphic(term1)
    original_count = term1.graphics_count()
    assert original_count >= 1

    json_str = term1.export_graphics_json()

    # Import into a new terminal
    term2 = Terminal(80, 24)
    assert term2.graphics_count() == 0
    restored = term2.import_graphics_json(json_str)
    assert restored == original_count
    assert term2.graphics_count() == original_count


def test_import_graphics_preserves_metadata():
    """Imported graphics should preserve placement metadata."""
    term1 = Terminal(80, 24)
    _create_sixel_graphic(term1)

    original = term1.graphics_at_row(0)[0]
    original_width = original.width
    original_height = original.height
    original_protocol = original.protocol
    original_display_mode = original.placement.display_mode

    json_str = term1.export_graphics_json()

    term2 = Terminal(80, 24)
    term2.import_graphics_json(json_str)

    restored = term2.graphics_at_row(0)[0]
    assert restored.width == original_width
    assert restored.height == original_height
    assert restored.protocol == original_protocol
    assert restored.placement.display_mode == original_display_mode


def test_import_clears_existing_graphics():
    """Importing should clear existing graphics first."""
    term = Terminal(80, 24)
    _create_sixel_graphic(term)
    assert term.graphics_count() >= 1

    # Export, then add another graphic
    json_str = term.export_graphics_json()
    _create_sixel_graphic(term)
    before_import = term.graphics_count()

    # Import should reset to the exported state
    restored = term.import_graphics_json(json_str)
    assert term.graphics_count() < before_import
    assert term.graphics_count() == restored


def test_import_invalid_json_raises_error():
    """Importing invalid JSON should raise an error."""
    term = Terminal(80, 24)
    try:
        term.import_graphics_json("not valid json")
        assert False, "Expected an error"
    except RuntimeError:
        pass


def test_export_import_preserves_pixel_data():
    """Pixel data should survive serialization round trip."""
    term1 = Terminal(80, 24)
    _create_sixel_graphic(term1)

    original = term1.graphics_at_row(0)[0]
    original_pixels = original.pixels()
    assert len(original_pixels) > 0

    json_str = term1.export_graphics_json()

    term2 = Terminal(80, 24)
    term2.import_graphics_json(json_str)

    restored = term2.graphics_at_row(0)[0]
    restored_pixels = restored.pixels()
    assert restored_pixels == original_pixels


def test_export_to_file_and_reimport():
    """Full workflow: export to file, read back, import."""
    term1 = Terminal(80, 24)
    _create_sixel_graphic(term1)

    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".json", delete=False, encoding="utf-8"
    ) as f:
        f.write(term1.export_graphics_json())
        tmp_path = f.name

    try:
        with open(tmp_path, encoding="utf-8") as f:
            json_str = f.read()

        term2 = Terminal(80, 24)
        count = term2.import_graphics_json(json_str)
        assert count >= 1
        assert term2.graphics_count() == count
    finally:
        Path(tmp_path).unlink()


def test_multiple_graphics_round_trip():
    """Multiple graphics of different types survive serialization."""
    term = Terminal(80, 24)

    # Add two sixel graphics
    _create_sixel_graphic(term)
    _create_sixel_graphic(term)
    original_count = term.graphics_count()
    assert original_count >= 2

    json_str = term.export_graphics_json()

    term2 = Terminal(80, 24)
    restored = term2.import_graphics_json(json_str)
    assert restored == original_count
    assert term2.graphics_count() == original_count
