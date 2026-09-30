#!/usr/bin/env python3
"""InlineImage construction and store reachability (QA-220).

InlineImage had no Python constructor and was not exported from the package
root, so the inline-image store could never receive its first image from
Python even though every store method was bound.
"""

import pytest
from par_term_emu_core_rust import InlineImage, Terminal


def make_image(image_id: str | None = "logo", position: tuple = (0, 0)) -> InlineImage:
    return InlineImage(
        "iterm2", "png", b"\x89PNG-bytes", 8, 4, position, 8, 2, image_id
    )


def test_construct_and_read_fields():
    img = make_image()
    assert img.id == "logo"
    assert img.protocol == "iterm2"
    assert img.format == "png"
    assert img.data == b"\x89PNG-bytes"
    assert img.width == 8
    assert img.height == 4
    assert img.position == (0, 0)
    assert img.display_cols == 8
    assert img.display_rows == 2


def test_id_defaults_to_none():
    img = make_image(image_id=None)
    assert img.id is None


def test_store_round_trip():
    term = Terminal(80, 24)
    img = make_image(position=(3, 1))
    term.add_inline_image(img)
    assert len(term.get_all_images()) == 1
    assert len(term.get_images_at(3, 1)) == 1
    assert len(term.get_images_at(0, 0)) == 0
    found = term.get_image_by_id("logo")
    assert found is not None
    assert found.position == (3, 1)
    assert term.delete_image("logo") is True
    assert len(term.get_all_images()) == 0


def test_add_rejects_unknown_protocol():
    term = Terminal(80, 24)
    img = InlineImage("nope", "png", b"x", 8, 4, (0, 0), 8, 2)
    with pytest.raises(ValueError):
        term.add_inline_image(img)


def test_add_rejects_unknown_format():
    term = Terminal(80, 24)
    img = InlineImage("iterm2", "webp", b"x", 8, 4, (0, 0), 8, 2)
    with pytest.raises(ValueError):
        term.add_inline_image(img)
