"""Per-consumer damage generations (ENH-025).

damage_generation()/dirty_rows_since() let independent consumers observe
edits without interfering: mark_clean() advances only the default consumer
that get_dirty_rows() serves.
"""

import pytest
from par_term_emu_core_rust import Terminal


@pytest.fixture
def term():
    return Terminal(20, 4)


def test_generation_advances_and_windows_edits(term):
    term.process(b"hello\r\nworld")
    gen0 = term.damage_generation()
    assert gen0 > 0
    assert term.dirty_rows_since(gen0) == []

    term.process(b"!")
    assert term.dirty_rows_since(gen0) == [1]
    assert term.get_dirty_rows() == [0, 1]


def test_mark_clean_does_not_hide_from_generation_consumer(term):
    term.process(b"hello\r\nworld")
    gen0 = term.damage_generation()
    term.process(b"!")

    term.mark_clean()
    assert term.get_dirty_rows() == []
    assert term.dirty_rows_since(gen0) == [1]


def test_screen_switch_dirties_all_rows_since_older_generation(term):
    term.process(b"hello")
    gen0 = term.damage_generation()

    term.process(b"\x1b[?1049h")
    assert term.dirty_rows_since(gen0) == [0, 1, 2, 3]
