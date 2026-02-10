#!/usr/bin/env python3
"""Tests for Unicode normalization support."""

from par_term_emu_core_rust import NormalizationForm, Terminal


def test_normalization_form_enum_values():
    """Test NormalizationForm enum members exist and have expected properties."""
    assert NormalizationForm.Disabled is not None
    assert NormalizationForm.NFC.name() == "NFC"
    assert NormalizationForm.NFD.name() == "NFD"
    assert NormalizationForm.NFKC.name() == "NFKC"
    assert NormalizationForm.NFKD.name() == "NFKD"
    assert NormalizationForm.Disabled.name() == "none"


def test_normalization_form_is_none():
    """Test is_none() method."""
    assert NormalizationForm.Disabled.is_none()
    assert not NormalizationForm.NFC.is_none()
    assert not NormalizationForm.NFD.is_none()


def test_normalization_form_repr():
    """Test __repr__ for NormalizationForm."""
    assert "NFC" in repr(NormalizationForm.NFC)
    assert "NFD" in repr(NormalizationForm.NFD)
    assert "Disabled" in repr(NormalizationForm.Disabled)


def test_default_normalization_is_nfc():
    """Test that Terminal defaults to NFC normalization."""
    term = Terminal(80, 24)
    form = term.normalization_form()
    assert form == NormalizationForm.NFC


def test_set_normalization_form():
    """Test setting different normalization forms."""
    term = Terminal(80, 24)

    for form in [
        NormalizationForm.Disabled,
        NormalizationForm.NFC,
        NormalizationForm.NFD,
        NormalizationForm.NFKC,
        NormalizationForm.NFKD,
    ]:
        term.set_normalization_form(form)
        assert term.normalization_form() == form


def test_nfc_composes_text():
    """Test that NFC normalization composes decomposed characters.

    With NFC, e + combining acute (U+0301) should be stored as é (U+00E9).
    """
    term = Terminal(80, 24)
    term.set_normalization_form(NormalizationForm.NFC)

    # Write decomposed e + combining acute accent
    term.process_str("e\u0301")

    # With NFC, the cell at (0,0) should contain precomposed é
    char = term.get_char(0, 0)
    assert char == "\u00e9", f"Expected precomposed é (U+00E9), got {repr(char)}"


def test_nfd_decomposes_text():
    """Test that NFD normalization decomposes precomposed characters.

    With NFD, é (U+00E9) should be stored as e + combining acute (U+0301).
    get_char returns the full grapheme (base + combining marks).
    """
    term = Terminal(80, 24)
    term.set_normalization_form(NormalizationForm.NFD)

    # Write precomposed é
    term.process_str("\u00e9")

    # With NFD, the cell should contain decomposed form: e + combining acute
    # get_char returns full grapheme, so we get the decomposed representation
    char = term.get_char(0, 0)
    assert char == "e\u0301", f"Expected decomposed e+acute, got {repr(char)}"
    # Cursor should be at col 1 (one cell used)
    assert term.cursor_position() == (1, 0)


def test_disabled_passthrough():
    """Test that Disabled normalization passes text through unchanged.

    With no normalization, e + combining acute stays decomposed (not composed).
    get_char returns the full grapheme, so we get the decomposed form.
    """
    term = Terminal(80, 24)
    term.set_normalization_form(NormalizationForm.Disabled)

    # Write decomposed form - 'e' followed by combining acute accent
    term.process_str("e\u0301")

    # With no normalization, the combining mark is added to 'e' cell
    # get_char returns full grapheme (base + combining)
    char = term.get_char(0, 0)
    assert char == "e\u0301", f"Expected decomposed e+acute, got {repr(char)}"


def test_nfkc_compatibility_normalization():
    """Test that NFKC replaces compatibility characters."""
    term = Terminal(80, 24)
    term.set_normalization_form(NormalizationForm.NFKC)

    # Write ﬁ ligature (U+FB01)
    term.process_str("\ufb01")

    # NFKC should decompose ﬁ into f + i
    char0 = term.get_char(0, 0)
    char1 = term.get_char(1, 0)
    assert char0 == "f", f"Expected 'f', got {repr(char0)}"
    assert char1 == "i", f"Expected 'i', got {repr(char1)}"


def test_ascii_unchanged_by_normalization():
    """Test that ASCII text is unaffected by any normalization form."""
    for form in [
        NormalizationForm.Disabled,
        NormalizationForm.NFC,
        NormalizationForm.NFD,
        NormalizationForm.NFKC,
        NormalizationForm.NFKD,
    ]:
        term = Terminal(80, 24)
        term.set_normalization_form(form)
        term.process_str("Hello")
        content = term.content()
        assert "Hello" in content, f"ASCII text lost with {form.name()} normalization"


def test_normalization_form_equality():
    """Test NormalizationForm equality comparison."""
    assert NormalizationForm.NFC == NormalizationForm.NFC
    assert NormalizationForm.NFC != NormalizationForm.NFD
    assert NormalizationForm.Disabled != NormalizationForm.NFC


def test_nfc_vs_disabled_composition():
    """Test that NFC composes while Disabled preserves decomposed form.

    Both store the combining mark with the base char, but NFC composes it.
    """
    # NFC: decomposed input -> composed storage
    term_nfc = Terminal(80, 24)
    term_nfc.set_normalization_form(NormalizationForm.NFC)
    term_nfc.process_str("e\u0301")
    nfc_char = term_nfc.get_char(0, 0)

    # Disabled: decomposed input -> decomposed storage
    term_disabled = Terminal(80, 24)
    term_disabled.set_normalization_form(NormalizationForm.Disabled)
    term_disabled.process_str("e\u0301")
    disabled_char = term_disabled.get_char(0, 0)

    # NFC should give precomposed é, Disabled should give decomposed e+acute
    assert nfc_char == "\u00e9", f"NFC: expected precomposed, got {repr(nfc_char)}"
    assert disabled_char == "e\u0301", (
        f"Disabled: expected decomposed, got {repr(disabled_char)}"
    )
    assert nfc_char != disabled_char


def test_nfc_hangul():
    """Test NFC properly composes Hangul syllables."""
    term = Terminal(80, 24)
    term.set_normalization_form(NormalizationForm.NFC)

    # Write precomposed Hangul syllable 한 (U+D55C) - should stay as-is under NFC
    hangul = "\ud55c"
    term.process_str(hangul)

    char = term.get_char(0, 0)
    assert char == hangul, f"Expected 한, got {repr(char)}"
