"""Terminal.encode_key — the shared key encoder exposed to Python (ENH-028).

Byte expectations mirror par-term-input's key encoding suite, so a Python
frontend, the par-term desktop app and the C FFI all produce identical
bytes for the same event.
"""

import pytest
from par_term_emu_core_rust import Terminal

# Key codes (TERM_KEY_* values; functional codes ARE the kitty codes).
CHAR = 1
TAB = 9
ENTER = 13
ESCAPE = 27
BACKSPACE = 127
UP = 57430
LEFT = 57428
F1 = 57376
F5 = 57380
PAGE_UP = 57432

# Modifier bits + side-info bit.
SHIFT, ALT, CTRL, SUPER, ALT_RIGHT = 1, 2, 4, 8, 64

# Option-key modes.
NORMAL, META, ESC = 0, 1, 2


@pytest.fixture
def term():
    return Terminal(80, 24)


def test_plain_and_control_characters(term):
    assert term.encode_key(CHAR, 0, ord("a")) == b"a"
    assert term.encode_key(CHAR, 0, ord("é")) == "é".encode()
    assert term.encode_key(CHAR, CTRL, ord("c")) == b"\x03"
    assert term.encode_key(CHAR, CTRL, ord(" ")) == b"\x00"


def test_functional_keys(term):
    assert term.encode_key(UP, 0) == b"\x1b[A"
    assert term.encode_key(LEFT, CTRL) == b"\x1b[1;5D"
    assert term.encode_key(ENTER, 0) == b"\r"
    assert term.encode_key(ENTER, SHIFT) == b"\n"
    assert term.encode_key(TAB, SHIFT) == b"\x1b[Z"
    assert term.encode_key(BACKSPACE, 0) == b"\x7f"
    assert term.encode_key(ESCAPE, 0) == b"\x1b"
    assert term.encode_key(F1, 0) == b"\x1bOP"
    assert term.encode_key(F5, SHIFT) == b"\x1b[15;2~"
    assert term.encode_key(PAGE_UP, CTRL) == b"\x1b[5;5~"


def test_super_is_not_an_xterm_modifier(term):
    # Cmd+Up encodes exactly as a bare Up (shortcuts are intercepted above
    # this layer); the side-info bit must not fabricate a parameter either.
    assert term.encode_key(UP, SUPER) == b"\x1b[A"
    assert term.encode_key(UP, ALT_RIGHT) == b"\x1b[A"


def test_option_key_modes(term):
    assert term.encode_key(CHAR, ALT, ord("f"), left_option=NORMAL) == b"f"
    assert term.encode_key(CHAR, ALT, ord("f"), left_option=META) == b"\xe6"
    assert term.encode_key(CHAR, ALT, ord("f"), left_option=ESC) == b"\x1bf"
    # Non-ASCII bases ESC-prefix in Meta too (no high bit without breaking
    # UTF-8).
    assert term.encode_key(CHAR, ALT, ord("é"), left_option=META) == b"\x1b\xc3\xa9"
    # Side selection: ALT_RIGHT routes to right_option.
    opts = {"left_option": NORMAL, "right_option": META}
    assert term.encode_key(CHAR, ALT, ord("a"), **opts) == b"a"
    assert term.encode_key(CHAR, ALT | ALT_RIGHT, ord("a"), **opts) == b"\xe1"
    # Defaults (0) are Normal passthrough.
    assert term.encode_key(CHAR, ALT, ord("a")) == b"a"


def test_modify_other_keys(term):
    term.process(b"\x1b[>4;2m")
    # Base codepoint, not the shifted glyph; Ctrl+letter included (modes
    # 1 and 2 share one rule set, matching par-term).
    assert term.encode_key(CHAR, CTRL | SHIFT, ord("1")) == b"\x1b[27;6;49~"
    assert term.encode_key(CHAR, CTRL, ord("c")) == b"\x1b[27;5;99~"
    # Shift-only exempt; non-ASCII base falls through to plain text.
    assert term.encode_key(CHAR, SHIFT, ord("1")) == b"1"
    assert term.encode_key(CHAR, CTRL, ord("é")) == "é".encode()
    # Reset restores the control byte.
    term.process(b"\x1b[>4m")
    assert term.encode_key(CHAR, CTRL, ord("c")) == b"\x03"


def test_kitty_disambiguate_mode(term):
    term.set_keyboard_flags(1)
    assert term.encode_key(CHAR, 0, ord("a")) == b"a"
    assert term.encode_key(CHAR, CTRL, ord("a")) == b"\x1b[97;5u"
    # Option modes must not leak into kitty — Alt reports as a modifier.
    assert term.encode_key(CHAR, ALT, ord("a"), left_option=META) == b"\x1b[97;3u"


def test_unknown_key_encodes_to_nothing(term):
    assert term.encode_key(0, 0) == b""
    assert term.encode_key(57388, 0) == b""


def test_kitty_astral_codepoint(term):
    term.process(b"\x1b[>1u")
    assert term.encode_key(CHAR, CTRL, 0x1D54F) == b"\x1b[120143;5u"


def test_kitty_unknown_key_encodes_to_nothing(term):
    term.set_keyboard_flags(1)
    assert term.encode_key(0, 0) == b""
    assert term.encode_key(57437, CTRL) == b""
