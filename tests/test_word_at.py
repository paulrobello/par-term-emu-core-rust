#!/usr/bin/env python3
"""Word extraction at display columns (QA-001).

`col` is a display column: a wide character's trailing spacer column
resolves to the character, and multi-char grapheme clusters (emoji + ZWJ,
combining marks) are kept whole.
"""

import pytest

from par_term_emu_core_rust import Terminal


def make_term(text: str) -> Terminal:
    term = Terminal(80, 24)
    term.process_str(text)
    return term


def test_get_word_at_wide_char_columns():
    term = make_term("日本語 word")
    for col in (0, 2, 4):
        assert term.get_word_at(col, 0) == "日本語", f"leading column {col}"


def test_get_word_at_wide_char_spacer_column_resolves_to_char():
    term = make_term("日本語 word")
    for col in (1, 3, 5):
        assert term.get_word_at(col, 0) == "日本語", f"spacer column {col}"


def test_get_word_at_beyond_wide_char_run():
    term = make_term("日本語 word")
    assert term.get_word_at(7, 0) == "word"
    assert term.get_word_at(6, 0) is None  # the space between
    assert term.get_word_at(30, 0) is None  # past the text


def test_get_word_at_emoji_zwj_line():
    term = make_term("👨‍💻 hi")
    assert term.get_word_at(3, 0) == "hi"
    assert term.get_word_at(4, 0) == "hi"
    # Emoji are not word characters.
    assert term.get_word_at(0, 0) is None
    assert term.get_word_at(1, 0) is None


def test_get_word_at_combining_mark_stays_in_word():
    # Decomposed e + combining acute; the terminal NFC-normalizes on write.
    term = make_term("café au")
    assert term.get_word_at(3, 0) == "café"


def test_get_word_at_default_word_chars():
    term = make_term("foo-bar baz")
    assert term.get_word_at(0, 0) == "foo-bar"
    assert term.get_word_at(0, 0, "_") == "foo"


def test_select_word_bounds_are_display_columns():
    term = make_term("日本語 word")
    term.select_word(2, 0)
    sel = term.get_selection()
    assert (sel.start, sel.end) == ((0, 0), (6, 0))


def test_select_word_returns_bounds_tuple():
    term = make_term("日本語 word")
    assert term.select_word(7, 0) == ((7, 0), (11, 0))
    assert term.select_word(6, 0) is None


def test_get_word_at_out_of_bounds():
    term = make_term("hi")
    assert term.get_word_at(1000, 0) is None
    assert term.get_word_at(0, 100) is None
