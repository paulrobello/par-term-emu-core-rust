#!/usr/bin/env python3
"""Test Sixel display and clear functionality."""

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow is required. Install with: pip install Pillow")
    sys.exit(1)

from par_term_emu_core_rust import Terminal


def rgb_to_sixel_color(r, g, b):
    """Convert RGB (0-255) to Sixel RGB format (0-100)."""
    return int(r * 100 / 255), int(g * 100 / 255), int(b * 100 / 255)


def image_to_sixel(image_path, max_width=None, max_height=None):
    """Convert an image to Sixel format."""
    img = Image.open(image_path)

    if img.mode != "RGB":
        img = img.convert("RGB")

    if max_width or max_height:
        img.thumbnail(
            (max_width or img.width, max_height or img.height), Image.Resampling.LANCZOS
        )

    # Quantize to 256 colors
    img = img.quantize(colors=256, method=Image.Quantize.MEDIANCUT).convert("RGB")

    width, height = img.size

    # Build color palette
    pixels = list(img.getdata())  # type: ignore[arg-type]
    unique_colors = list(set(pixels))[:256]
    color_to_index = {color: idx for idx, color in enumerate(unique_colors)}

    # Start Sixel sequence
    sixel_data = ["\x1bP0;0;0;q"]
    sixel_data.append(f'"1;1;{width};{height}')

    # Define color palette
    for idx, (r, g, b) in enumerate(unique_colors):
        sr, sg, sb = rgb_to_sixel_color(r, g, b)
        sixel_data.append(f"#{idx};2;{sr};{sg};{sb}")

    # Process image in strips of 6 pixels high
    for strip_y in range(0, height, 6):
        if strip_y != 0:
            sixel_data.append("-")

        for color_idx in range(len(unique_colors)):
            sixel_data.append(f"#{color_idx}")

            run_char: str | None = None
            run_len = 0

            def flush_run():
                nonlocal run_char, run_len
                # Guard against None so type checkers know run_char is str below
                if run_len == 0 or run_char is None:
                    return
                if run_len == 1:
                    sixel_data.append(run_char)
                else:
                    sixel_data.append(f"!{run_len}{run_char}")
                run_char = None
                run_len = 0

            for x in range(width):
                sixel_value = 0

                for bit in range(6):
                    y = strip_y + bit
                    if y < height:
                        pixel_idx = y * width + x
                        pixel_color = pixels[pixel_idx]
                        if color_to_index[pixel_color] == color_idx:
                            sixel_value |= 1 << bit

                sixel_char = chr((sixel_value if sixel_value > 0 else 0) + 0x3F)
                if run_char is None:
                    run_char = sixel_char
                    run_len = 1
                elif sixel_char == run_char:
                    run_len += 1
                else:
                    flush_run()
                    run_char = sixel_char
                    run_len = 1

            flush_run()
            sixel_data.append("$")

    sixel_data.append("\x1b\\")
    return "".join(sixel_data)


def test_sixel_display():
    """Test Sixel display and clear command."""
    print("=" * 60)
    print("Sixel Display and Clear Test")
    print("=" * 60)

    # Create terminal
    term = Terminal(80, 24)

    # Convert and send image
    print("\n1. Converting image to Sixel...")
    image_path = Path("snake.tiff")

    if not image_path.exists():
        print(f"Error: {image_path} not found")
        return False

    sixel_str = image_to_sixel(image_path, max_width=400, max_height=200)
    print(f"   Sixel sequence length: {len(sixel_str)} bytes")

    print("\n2. Sending Sixel to terminal...")
    term.process_str(sixel_str)

    graphics_count = term.graphics_count()
    print(f"   ✓ Graphics stored: {graphics_count}")

    if graphics_count == 0:
        print("   ✗ FAILED: No graphics were stored")
        return False

    # Get graphic details
    graphics = term.graphics_at_row(0)
    if graphics:
        graphic = graphics[0]
        print(f"   ✓ Graphic position: {graphic.position}")
        print(f"   ✓ Graphic size: {graphic.width}x{graphic.height}")

    # Test clear command (ED 2)
    print("\n3. Sending clear command (ESC[2J)...")
    term.process_str("\x1b[2J")

    graphics_after = term.graphics_count()
    print(f"   Graphics after clear: {graphics_after}")

    if graphics_after == 0:
        print("   ✓ Clear command successfully cleared graphics!")
    else:
        print(
            f"   ✗ FAILED: Graphics still present after clear (count={graphics_after})"
        )
        return False

    # Test sending graphics again
    print("\n4. Sending graphics again after clear...")
    term.process_str(sixel_str)

    graphics_count2 = term.graphics_count()
    print(f"   ✓ Graphics stored: {graphics_count2}")

    if graphics_count2 == 0:
        print("   ✗ FAILED: Graphics not stored after second send")
        return False

    # Test manual clear_graphics()
    print("\n5. Testing manual clear_graphics()...")
    term.clear_graphics()

    graphics_after2 = term.graphics_count()
    print(f"   Graphics after manual clear: {graphics_after2}")

    if graphics_after2 == 0:
        print("   ✓ Manual clear_graphics() works!")
    else:
        print(f"   ✗ FAILED: Manual clear failed (count={graphics_after2})")
        return False

    print("\n" + "=" * 60)
    print("✓ All Sixel display tests passed!")
    print("=" * 60)
    return True


if __name__ == "__main__":
    success = test_sixel_display()
    sys.exit(0 if success else 1)
