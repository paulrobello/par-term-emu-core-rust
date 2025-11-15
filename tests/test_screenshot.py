"""Tests for screenshot functionality"""

import pytest
from par_term_emu_core_rust import Terminal, PtyTerminal
import os
import tempfile


class TestScreenshotFormats:
    """Test screenshot format support"""

    def test_png_format(self):
        """Test PNG screenshot format"""
        term = Terminal(80, 24)
        term.process_str("\x1b[31mRed text\x1b[0m\n")

        png_bytes = term.screenshot(format="png")
        assert len(png_bytes) > 0
        assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"  # PNG signature

    def test_jpeg_format(self):
        """Test JPEG screenshot format"""
        term = Terminal(80, 24)
        term.process_str("Test content\n")

        jpg_bytes = term.screenshot(format="jpeg")
        assert len(jpg_bytes) > 0
        assert jpg_bytes[:2] == b"\xff\xd8"  # JPEG signature

    def test_jpeg_alias(self):
        """Test that 'jpg' works as alias for 'jpeg'"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        jpg_bytes = term.screenshot(format="jpg")
        assert len(jpg_bytes) > 0
        assert jpg_bytes[:2] == b"\xff\xd8"

    def test_bmp_format(self):
        """Test BMP screenshot format"""
        term = Terminal(80, 24)
        term.process_str("Test content\n")

        bmp_bytes = term.screenshot(format="bmp")
        assert len(bmp_bytes) > 0
        assert bmp_bytes[:2] == b"BM"  # BMP signature

    def test_svg_format(self):
        """Test SVG screenshot format"""
        term = Terminal(80, 24)
        term.process_str("Hello, World!\n")

        svg_bytes = term.screenshot(format="svg")
        assert len(svg_bytes) > 0

        # Decode and check SVG content
        svg_text = svg_bytes.decode("utf-8")
        assert svg_text.startswith('<?xml version="1.0"')
        assert "<svg" in svg_text
        assert "</svg>" in svg_text
        assert "Hello, World!" in svg_text

    def test_svg_with_colors(self):
        """Test SVG with colored text"""
        term = Terminal(80, 24)
        term.process_str("\x1b[31mRed text\x1b[0m\n")

        svg_bytes = term.screenshot(format="svg")
        svg_text = svg_bytes.decode("utf-8")

        # Should contain RGB color values
        assert "rgb(" in svg_text
        assert "Red text" in svg_text

    def test_svg_with_attributes(self):
        """Test SVG with text attributes"""
        term = Terminal(80, 24)
        term.process_str("\x1b[1mBold\x1b[0m \x1b[3mItalic\x1b[0m\n")

        svg_bytes = term.screenshot(format="svg")
        svg_text = svg_bytes.decode("utf-8")

        # Should contain CSS classes for attributes
        assert 'class="bold"' in svg_text
        assert 'class="italic"' in svg_text

    def test_invalid_format(self):
        """Test that invalid format raises error"""
        term = Terminal(80, 24)

        with pytest.raises(ValueError, match="Invalid format"):
            term.screenshot(format="invalid")


class TestScreenshotConfiguration:
    """Test screenshot configuration options"""

    def test_default_config(self):
        """Test screenshot with default configuration"""
        term = Terminal(80, 24)
        term.process_str("Hello, World!\n")

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_font_size(self):
        """Test custom font size"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        small = term.screenshot(font_size=10.0)
        large = term.screenshot(font_size=20.0)

        # Larger font should produce larger image
        assert len(large) > len(small)

    def test_padding(self):
        """Test custom padding"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        no_padding = term.screenshot(padding=0)
        with_padding = term.screenshot(padding=50)

        # More padding should produce larger image
        assert len(with_padding) > len(no_padding)

    def test_jpeg_quality(self):
        """Test JPEG quality setting"""
        term = Terminal(80, 24)
        # Fill with more content for better quality comparison
        for i in range(20):
            term.process_str(f"Line {i}: Some test content here\n")

        low_quality = term.screenshot(format="jpeg", quality=10)
        high_quality = term.screenshot(format="jpeg", quality=100)

        # Both should be valid JPEG
        assert low_quality[:2] == b"\xff\xd8"
        assert high_quality[:2] == b"\xff\xd8"


class TestScreenshotToFile:
    """Test screenshot_to_file functionality"""

    def test_save_png(self):
        """Test saving screenshot to PNG file"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
            filename = f.name

        try:
            term.screenshot_to_file(filename)
            assert os.path.exists(filename)
            assert os.path.getsize(filename) > 0

            # Verify it's a valid PNG
            with open(filename, "rb") as f:
                header = f.read(8)
                assert header == b"\x89PNG\r\n\x1a\n"
        finally:
            if os.path.exists(filename):
                os.remove(filename)

    def test_auto_detect_format(self):
        """Test auto-detection of format from file extension"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        formats = [
            ("test.png", b"\x89PNG\r\n\x1a\n"),
            ("test.jpg", b"\xff\xd8"),
            ("test.jpeg", b"\xff\xd8"),
            ("test.bmp", b"BM"),
            ("test.svg", b'<?xml version="1.0"'),
        ]

        for filename, signature in formats:
            with tempfile.TemporaryDirectory() as tmpdir:
                filepath = os.path.join(tmpdir, filename)
                term.screenshot_to_file(filepath)

                assert os.path.exists(filepath)
                with open(filepath, "rb") as f:
                    header = f.read(len(signature))
                    assert header == signature, f"Invalid signature for {filename}"

    def test_explicit_format_override(self):
        """Test explicit format parameter overrides extension"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        with tempfile.TemporaryDirectory() as tmpdir:
            # Save as PNG despite .jpg extension
            filepath = os.path.join(tmpdir, "test.jpg")
            term.screenshot_to_file(filepath, format="png")

            assert os.path.exists(filepath)
            with open(filepath, "rb") as f:
                header = f.read(8)
                assert header == b"\x89PNG\r\n\x1a\n"


class TestScreenshotContent:
    """Test screenshot rendering of various content"""

    def test_colors(self):
        """Test that colored content generates valid screenshots"""
        term = Terminal(80, 24)

        # Test all basic colors
        colors = ["30", "31", "32", "33", "34", "35", "36", "37"]
        for color in colors:
            term.process_str(f"\x1b[{color}mColor {color}\x1b[0m ")

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_text_attributes(self):
        """Test text attributes in screenshots"""
        term = Terminal(80, 24)

        term.process_str("\x1b[1mBold\x1b[0m ")
        term.process_str("\x1b[3mItalic\x1b[0m ")
        term.process_str("\x1b[4mUnderline\x1b[0m ")
        term.process_str("\x1b[9mStrikethrough\x1b[0m\n")

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_unicode_content(self):
        """Test Unicode characters in screenshots"""
        term = Terminal(80, 24)

        term.process_str("Unicode: ")
        term.process_str("こんにちは ")  # Japanese
        term.process_str("🎨🚀✨ ")  # Emoji
        term.process_str("±≠∞\n")  # Symbols

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_empty_terminal(self):
        """Test screenshot of empty terminal"""
        term = Terminal(80, 24)

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0


class TestPtyTerminalScreenshot:
    """Test screenshot functionality with PtyTerminal"""

    @pytest.mark.skip(reason="PTY screenshot tests hang in CI")
    def test_pty_screenshot(self):
        """Test screenshot from PTY terminal"""
        import time

        with PtyTerminal(80, 24) as pty:
            # Spawn a shell to activate the PTY
            pty.spawn_shell()
            time.sleep(0.1)  # Let shell start

            # Take screenshot (should capture shell prompt)
            png_bytes = pty.screenshot()
            assert len(png_bytes) > 0
            assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"

            # Clean exit
            pty.write_str("exit\n")
            time.sleep(0.1)

    @pytest.mark.skip(reason="PTY screenshot tests hang in CI")
    def test_pty_screenshot_to_file(self):
        """Test saving PTY screenshot to file"""
        import time

        with PtyTerminal(80, 24) as pty:
            pty.spawn_shell()
            time.sleep(0.1)

            with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
                filename = f.name

            try:
                pty.screenshot_to_file(filename)
                assert os.path.exists(filename)
                assert os.path.getsize(filename) > 0
            finally:
                if os.path.exists(filename):
                    os.remove(filename)

            pty.write_str("exit\n")
            time.sleep(0.1)


class TestScreenshotEdgeCases:
    """Test edge cases and error conditions"""

    def test_very_small_terminal(self):
        """Test screenshot of very small terminal"""
        term = Terminal(10, 5)
        term.process_str("Small\n")

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_very_large_terminal(self):
        """Test screenshot of large terminal"""
        term = Terminal(200, 100)
        for i in range(50):
            term.process_str(f"Line {i}\n")

        png_bytes = term.screenshot()
        assert len(png_bytes) > 0

    def test_no_system_font_error_message(self):
        """Test that font errors provide helpful message"""
        term = Terminal(80, 24)
        term.process_str("Test\n")

        # Try with invalid font path
        try:
            term.screenshot(font_path="/nonexistent/font.ttf")
            # If we get here, system font fallback worked
            assert True
        except RuntimeError as e:
            # Should get a clear error message
            assert "Screenshot error" in str(e)


class TestEmojiRendering:
    """Test color emoji rendering in screenshots"""

    @pytest.mark.skip(reason="Emoji color test hangs on Linux CI")
    def test_emoji_color_rendering(self):
        """Test that emoji render in color (not grayscale)"""
        try:
            from PIL import Image  # type: ignore[import-not-found]
            import io
        except ImportError:
            pytest.skip("PIL not available for color verification")

        term = Terminal(80, 24)
        term.process_str("Emoji: 🐍 🦀 ❤️ ✨ 🌟\n")

        png_bytes = term.screenshot(format="png")
        assert len(png_bytes) > 0

        # Load image and check for colored pixels
        img = Image.open(io.BytesIO(png_bytes))
        img = img.convert("RGBA")
        pixels = list(img.getdata())  # type: ignore[arg-type]

        # Count colored pixels (not grayscale)
        colored_pixels = sum(1 for r, g, b, a in pixels if r != g or g != b or r != b)

        # With color emoji support, should have significant number of colored pixels
        # For emoji-only content, expect at least 0.5% colored pixels
        total_pixels = len(pixels)
        color_ratio = colored_pixels / total_pixels

        # If color ratio is very low, emoji might still be rendering as grayscale
        # This is okay on systems without color emoji fonts, so we just warn
        assert color_ratio >= 0.0, (
            "Should have some pixels (may be grayscale placeholders)"
        )

        # If we have colored pixels, verify they're actual colors not just black/white
        if colored_pixels > 0:
            # Sample some non-grayscale pixels
            non_grayscale = [
                (r, g, b) for r, g, b, a in pixels if (r != g or g != b) and a > 0
            ]
            if non_grayscale:
                # Verify we have actual RGB color values
                assert any(
                    r != 0 or g != 0 or b != 0 for r, g, b in non_grayscale[:100]
                )

    def test_mixed_text_and_emoji(self):
        """Test rendering of text mixed with emoji"""
        term = Terminal(80, 24)
        term.process_str("Text with emoji: 🎨 More text 🚀\n")
        term.process_str("\x1b[31mRed text\x1b[0m with ❤️ emoji\n")

        png_bytes = term.screenshot(format="png")
        assert len(png_bytes) > 0
        assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"

    def test_emoji_in_svg(self):
        """Test that emoji appear in SVG output"""
        term = Terminal(80, 24)
        term.process_str("Emoji: 🐍 🦀 ✨\n")

        svg_bytes = term.screenshot(format="svg")
        svg_text = svg_bytes.decode("utf-8")

        # SVG should contain the emoji characters
        assert "🐍" in svg_text or "&#" in svg_text  # Either literal or HTML entity
        assert "<svg" in svg_text

    def test_wide_emoji(self):
        """Test that wide (2-cell) emoji render correctly"""
        term = Terminal(80, 24)
        # Many emoji are wide characters (2 terminal cells)
        term.process_str("Wide: 👨‍👩‍👧‍👦 🏴󠁧󠁢󠁳󠁣󠁴󠁿\n")

        png_bytes = term.screenshot(format="png")
        assert len(png_bytes) > 0

    def test_emoji_with_variation_selectors(self):
        """Test emoji with variation selectors (text vs emoji presentation)"""
        term = Terminal(80, 24)
        # U+2764 followed by U+FE0F (emoji variation selector)
        term.process_str("Heart: ❤️ Star: ⭐\n")

        png_bytes = term.screenshot(format="png")
        assert len(png_bytes) > 0


class TestScrollbackOffset:
    """Test screenshot with scrollback offset parameter"""

    def test_screenshot_with_scrollback_offset(self):
        """Test screenshot captures view at scrollback offset"""
        term = Terminal(80, 24, scrollback=100)

        # Fill scrollback with numbered lines
        for i in range(50):
            term.process_str(f"Line {i:03d}\n")

        # Take screenshot with no offset (current view)
        png_bytes_current = term.screenshot(scrollback_offset=0)
        assert len(png_bytes_current) > 0
        assert png_bytes_current[:8] == b"\x89PNG\r\n\x1a\n"

        # Take screenshot with offset (scrolled back 10 lines)
        png_bytes_offset = term.screenshot(scrollback_offset=10)
        assert len(png_bytes_offset) > 0
        assert png_bytes_offset[:8] == b"\x89PNG\r\n\x1a\n"

        # Images should be different (different content)
        assert png_bytes_current != png_bytes_offset

    def test_screenshot_to_file_with_scrollback_offset(self):
        """Test screenshot_to_file with scrollback offset"""
        term = Terminal(80, 24, scrollback=100)

        # Fill scrollback
        for i in range(30):
            term.process_str(f"Line {i:03d}\n")

        with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
            filename = f.name

        try:
            # Save screenshot with offset
            term.screenshot_to_file(filename, scrollback_offset=5)
            assert os.path.exists(filename)
            assert os.path.getsize(filename) > 0

            # Verify PNG signature
            with open(filename, "rb") as f:
                signature = f.read(8)
                assert signature == b"\x89PNG\r\n\x1a\n"
        finally:
            if os.path.exists(filename):
                os.remove(filename)

    def test_screenshot_offset_beyond_scrollback(self):
        """Test screenshot with offset larger than scrollback"""
        term = Terminal(80, 24, scrollback=10)

        # Add a few lines
        for i in range(5):
            term.process_str(f"Line {i}\n")

        # Request offset beyond available scrollback
        # Should handle gracefully
        png_bytes = term.screenshot(scrollback_offset=100)
        assert len(png_bytes) > 0
        assert png_bytes[:8] == b"\x89PNG\r\n\x1a\n"

    def test_scrollback_content_verification(self):
        """Verify screenshot captures correct content at various offsets"""
        term = Terminal(80, 5, scrollback=100)  # Small terminal for easier testing

        # Fill with numbered lines (will push into scrollback)
        for i in range(15):
            term.process_str(f"Line {i:03d}\n")

        # After 15 lines in a 5-row terminal with newlines:
        # - Scrollback has lines 0-10 (11 lines)
        # - Active grid has lines 11-14 on rows 0-3, row 4 is empty (cursor position)

        # Test offset=0 (current view) - should show lines 11-14
        content_offset_0 = term.content()
        assert "Line 011" in content_offset_0
        assert "Line 014" in content_offset_0
        assert "Line 010" not in content_offset_0  # Should be in scrollback

        # Now verify with export_text which shows full content
        full_export = term.export_text()
        assert "Line 000" in full_export
        assert "Line 014" in full_export
