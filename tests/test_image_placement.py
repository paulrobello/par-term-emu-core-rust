"""Tests for image placement metadata parsing and exposure."""

from par_term_emu_core_rust import Terminal, ImagePlacement, ImageDimension


def test_sixel_graphics_have_default_placement():
    """Sixel graphics should have a default inline placement."""
    term = Terminal(80, 24)

    # Build a 2x6 solid red column using explicit raster size
    sixel = (
        "\x1bPq"  # DCS Sixel start
        '"1;1;2;6'  # raster: pan=1, pad=1, width=2, height=6
        "#0;2;100;0;0"  # define color 0 = red
        "#0"  # select color 0
        "~~"  # two columns filled
        "\x1b\\"  # ST
    )

    term.process_str(sixel)
    assert term.graphics_count() >= 1

    g = term.graphics_at_row(0)[0]

    # Sixel should have default placement
    assert g.placement is not None
    assert g.placement.display_mode == "inline"
    assert g.placement.preserve_aspect_ratio is True
    assert g.placement.z_index == 0
    assert g.placement.x_offset == 0
    assert g.placement.y_offset == 0
    assert g.placement.columns is None
    assert g.placement.rows is None


def test_placement_requested_dimensions_default_auto():
    """Default placement should have auto dimensions."""
    term = Terminal(80, 24)

    sixel = '\x1bPq"1;1;2;6#0;2;100;0;0#0~~\x1b\\'

    term.process_str(sixel)
    g = term.graphics_at_row(0)[0]

    # Default dimensions should be auto
    assert g.placement.requested_width.is_auto()
    assert g.placement.requested_height.is_auto()
    assert g.placement.requested_width.unit == "auto"
    assert g.placement.requested_height.unit == "auto"


def test_image_dimension_repr():
    """ImageDimension should have a useful repr."""
    term = Terminal(80, 24)

    sixel = '\x1bPq"1;1;2;6#0;2;100;0;0#0~~\x1b\\'

    term.process_str(sixel)
    g = term.graphics_at_row(0)[0]

    # Auto dimensions should show "auto" in repr
    assert "auto" in repr(g.placement.requested_width).lower()


def test_image_placement_repr():
    """ImagePlacement should have a useful repr."""
    term = Terminal(80, 24)

    sixel = '\x1bPq"1;1;2;6#0;2;100;0;0#0~~\x1b\\'

    term.process_str(sixel)
    g = term.graphics_at_row(0)[0]

    repr_str = repr(g.placement)
    assert "inline" in repr_str
    assert "preserve_aspect_ratio" in repr_str


def test_graphic_repr_unchanged():
    """Graphic repr should still work (backward compat)."""
    term = Terminal(80, 24)

    sixel = '\x1bPq"1;1;2;6#0;2;100;0;0#0~~\x1b\\'

    term.process_str(sixel)
    g = term.graphics_at_row(0)[0]

    repr_str = repr(g)
    assert "Graphic(" in repr_str
    assert "sixel" in repr_str


def test_image_placement_class_importable():
    """ImagePlacement and ImageDimension should be importable."""
    assert ImagePlacement is not None
    assert ImageDimension is not None
