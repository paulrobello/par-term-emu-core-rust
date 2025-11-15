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
