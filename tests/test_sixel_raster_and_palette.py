from par_term_emu_core_rust import Terminal


def test_sixel_raster_attributes_and_palette_parsing():
    """Ensure '"' raster attributes and '#' palette commands are parsed correctly.

    This covers a regression where parameter buffers were not flushed when a new
    command ('#', '"', '!') began, corrupting the previous command (e.g. raster
    attributes) and leading to distorted rendering (horizontal striping).
    """

    term = Terminal(80, 24)

    # Build a 2x6 solid red column using explicit raster size
    sixel = (
        "\x1bPq"  # DCS Sixel start
        '"1;1;2;6'  # raster: pan=1, pad=1, width=2, height=6
        "#0;2;100;0;0"  # define color 0 = red (RGB 100%,0%,0%)
        "#0"  # select color 0
        "~~"  # two columns filled (each '~' sets all 6 bits)
        "\x1b\\"  # ST
    )

    term.process_str(sixel)

    # Expect exactly one graphic and it should match the declared raster size
    assert term.graphics_count() >= 1
    g = term.graphics_at_row(0)[0]
    assert (g.width, g.height) == (2, 6)

    # Top-left pixel should be red with alpha 255
    r, g_, b, a = g.get_pixel(0, 0)
    assert a == 255 and r > 200 and g_ < 50 and b < 50


def test_sixel_original_dimensions_exposed():
    """Verify that original_width and original_height are exposed on Graphic objects."""

    term = Terminal(80, 24)

    # Build a 4x12 image (2 sixel characters wide, 2 sixel rows tall)
    sixel = (
        "\x1bPq"  # DCS Sixel start
        '"1;1;4;12'  # raster: pan=1, pad=1, width=4, height=12
        "#0;2;0;100;0"  # define color 0 = green (RGB 0%,100%,0%)
        "#0"  # select color 0
        "~~~~"  # four columns filled (first 6 rows)
        "-"  # new line (move down 6 pixels)
        "~~~~"  # four columns filled (next 6 rows)
        "\x1b\\"  # ST
    )

    term.process_str(sixel)

    assert term.graphics_count() >= 1
    g = term.graphics_at_row(0)[0]

    # Current dimensions should match original
    assert g.width == 4
    assert g.height == 12
    assert g.original_width == 4
    assert g.original_height == 12

    # Verify repr includes original_size
    repr_str = repr(g)
    assert "original_size=4x12" in repr_str


def test_sixel_limits_python_api():
    """Ensure Python get_sixel_limits/set_sixel_limits work and clamp safely."""

    term = Terminal(80, 24)

    # Defaults should be reasonable (bounded, non-zero)
    max_w, max_h, max_repeat = term.get_sixel_limits()
    assert 1 <= max_w <= 4096
    assert 1 <= max_h <= 4096
    assert 1 <= max_repeat <= 10_000

    # Tighten limits and verify they are applied
    term.set_sixel_limits(512, 256, 2000)
    max_w2, max_h2, max_repeat2 = term.get_sixel_limits()
    assert (max_w2, max_h2, max_repeat2) == (512, 256, 2000)

    # Ask for excessive limits; they should be clamped instead of crashing
    term.set_sixel_limits(10_000, 10_000, 1_000_000)
    max_w3, max_h3, max_repeat3 = term.get_sixel_limits()
    assert 1 <= max_w3 <= 4096
    assert 1 <= max_h3 <= 4096
    assert 1 <= max_repeat3 <= 10_000


def test_sixel_graphics_limit_python_api():
    """Ensure Python graphics limit API works and enforces a cap."""

    term = Terminal(80, 24)

    # Default should be within hard bounds
    default_limit = term.get_sixel_graphics_limit()
    assert 1 <= default_limit <= 1024

    # Set a very small limit and emit multiple graphics
    term.set_sixel_graphics_limit(2)
    limit = term.get_sixel_graphics_limit()
    assert limit == 2

    sixel = '\x1bPq"1;1;2;6#0;2;100;0;0#0~~\x1b\\'

    # Emit three tiny graphics; only 2 should be retained and at least one dropped
    term.process_str(sixel)
    term.process_str(sixel)
    term.process_str(sixel)

    assert term.graphics_count() <= 2
    assert term.get_dropped_sixel_graphics() >= 1

    # Stats API should reflect the same information
    stats = term.get_sixel_stats()
    assert stats["max_width_px"] >= 1
    assert stats["max_height_px"] >= 1
    assert stats["max_repeat"] >= 1
    assert stats["max_graphics"] == 2
    assert stats["current_graphics"] <= 2
    assert stats["dropped_graphics"] >= 1
