"""ARC-002: the `TerminalClipboard` facade delegates to the terminal it wraps.

Every facade call must act on the same state the flat `Terminal` methods see,
in both directions, so the two spellings can coexist during migration.
"""

import pytest
from par_term_emu_core_rust import Terminal, TerminalClipboard


def test_facade_writes_are_visible_through_flat_methods() -> None:
    term = Terminal(80, 24)
    clip = TerminalClipboard(term)
    clip.add("clipboard", "hello", label="greeting")
    clip.set("world", slot="primary")

    history = term.get_clipboard_history("clipboard")
    assert [(e.content, e.label) for e in history] == [("hello", "greeting")]
    assert term.get_clipboard_from_slot("primary") == "world"


def test_flat_writes_are_visible_through_facade() -> None:
    term = Terminal(80, 24)
    term.add_to_clipboard_history("clipboard", "one")
    term.add_to_clipboard_history("clipboard", "two words")
    clip = TerminalClipboard(term)

    assert [e.content for e in clip.history("clipboard")] == ["one", "two words"]
    latest = clip.latest("clipboard")
    assert latest is not None and latest.content == "two words"
    assert clip.get() == term.get_clipboard_from_slot()
    assert [e.content for e in clip.search("words")] == ["two words"]
    assert clip.latest("primary") is None


def test_clear_one_slot_or_all() -> None:
    term = Terminal(80, 24)
    clip = TerminalClipboard(term)
    clip.add("clipboard", "a")
    clip.add("primary", "b")

    clip.clear("clipboard")
    assert term.get_clipboard_history("clipboard") == []
    assert len(term.get_clipboard_history("primary")) == 1

    clip.clear()
    assert term.get_clipboard_history("primary") == []


def test_terminal_property_returns_the_wrapped_terminal() -> None:
    term = Terminal(80, 24)
    assert TerminalClipboard(term).terminal is term


def test_invalid_slot_raises_value_error() -> None:
    clip = TerminalClipboard(Terminal(80, 24))
    with pytest.raises(ValueError):
        clip.history("nope")
    with pytest.raises(ValueError):
        clip.add("nope", "x")
