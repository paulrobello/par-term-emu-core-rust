#!/usr/bin/env python3
"""Color conversion consistency (QA-009).

rgb_to_hsl/hsl_to_rgb delegate to the canonical color_utils
implementations; these tests pin the documented 0-100 scale and the
round-trip property, including the previously broken achromatic case.
"""

import pytest
from par_term_emu_core_rust import hsl_to_rgb, rgb_to_hsl


def test_rgb_to_hsl_red_documented_scale():
    h, s, l = rgb_to_hsl((255, 0, 0))
    assert h == pytest.approx(0.0, abs=1e-3)
    assert s == pytest.approx(100.0, abs=1e-3)
    assert l == pytest.approx(50.0, abs=1e-3)


def test_rgb_to_hsl_achromatic_lightness_scale():
    # White previously returned l=1.0 instead of 100.0.
    h, s, l = rgb_to_hsl((255, 255, 255))
    assert (h, s) == (0.0, 0.0)
    assert l == pytest.approx(100.0, abs=1e-3)

    h, s, l = rgb_to_hsl((0, 0, 0))
    assert l == pytest.approx(0.0, abs=1e-3)

    h, s, l = rgb_to_hsl((128, 128, 128))
    assert l == pytest.approx(128 / 255 * 100, abs=0.5)


def test_hsl_round_trip():
    for r in range(0, 256, 17):
        for g in range(0, 256, 17):
            for b in range(0, 256, 17):
                h, s, l = rgb_to_hsl((r, g, b))
                r2, g2, b2 = hsl_to_rgb(h, s, l)
                assert abs(r - r2) <= 1, f"r round-trip {(r, g, b)} -> {(r2, g2, b2)}"
                assert abs(g - g2) <= 1, f"g round-trip {(r, g, b)} -> {(r2, g2, b2)}"
                assert abs(b - b2) <= 1, f"b round-trip {(r, g, b)} -> {(r2, g2, b2)}"


def test_hsl_to_rgb_pure_green():
    assert hsl_to_rgb(120, 100, 50) == (0, 255, 0)
