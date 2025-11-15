#!/usr/bin/env python3
"""
Display images in Sixel format in the terminal.

This utility converts images to Sixel format and displays them.
Requires PIL (Pillow) for image processing.

Usage:
    python display_image_sixel.py <image_file>
    python display_image_sixel.py <image_file> --width 80 --height 24
"""

import sys
import os
import argparse
import shutil
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    print("Error: Pillow is required. Install with: pip install Pillow")
    sys.exit(1)


def rgb_to_sixel_color(r, g, b):
    """Convert RGB (0-255) to Sixel RGB format (0-100)."""
    return int(r * 100 / 255), int(g * 100 / 255), int(b * 100 / 255)


def image_to_sixel(image_path, max_width=None, max_height=None):
    """
    Convert an image to Sixel format.

    Args:
        image_path: Path to the image file
        max_width: Maximum width in pixels (None = no limit)
        max_height: Maximum height in pixels (None = no limit)

    Returns:
        String containing Sixel data
    """
    # Load image
    img = Image.open(image_path)

    # Convert to RGB if needed
    if img.mode != 'RGB':
        img = img.convert('RGB')

    # Resize if requested (fit within bounding box while preserving aspect ratio)
    if max_width or max_height:
        bound_w = max_width or img.width
        bound_h = max_height or img.height
        img.thumbnail((int(bound_w), int(bound_h)), Image.Resampling.LANCZOS)

    # Quantize image to at most 256 colors to ensure all pixels map to palette
    # This prevents KeyError when looking up pixel colors
    img = img.quantize(colors=256, method=Image.Quantize.MEDIANCUT).convert('RGB')

    width, height = img.size

    # Build color palette (simple: use actual colors, up to 256)
    pixels = list(img.getdata())
    unique_colors = list(set(pixels))[:256]  # Limit to 256 colors
    color_to_index = {color: idx for idx, color in enumerate(unique_colors)}

    # Start Sixel sequence
    # DCS P1 ; P2 ; P3 ; q
    # P1=0 (aspect ratio 2:1), P2=0 (pixel mode), P3=0 (ignored)
    # Explicitly request opaque background (P2=0) to avoid transparency artifacts
    sixel_data = ["\x1bP0;0;0;q"]  # DCS P1;P2;P3;q

    # Set raster attributes (optional but recommended)
    sixel_data.append(f'"1;1;{width};{height}')

    # Define color palette
    for idx, (r, g, b) in enumerate(unique_colors):
        sr, sg, sb = rgb_to_sixel_color(r, g, b)
        sixel_data.append(f"#{idx};2;{sr};{sg};{sb}")

    # Process image in strips of 6 pixels high
    for strip_y in range(0, height, 6):
        # Move to next 6-pixel row AFTER finishing the previous one
        if strip_y != 0:
            sixel_data.append("-")

        for color_idx in range(len(unique_colors)):
            sixel_data.append(f"#{color_idx}")  # Select color

            # Run-length encode across columns. Use '?' for empty columns so
            # horizontal alignment is preserved across color planes.
            run_char = None
            run_len = 0

            def flush_run():
                nonlocal run_char, run_len
                if run_len == 0:
                    return
                if run_len == 1:
                    sixel_data.append(run_char)
                else:
                    sixel_data.append(f"!{run_len}{run_char}")
                run_char = None
                run_len = 0

            for x in range(width):
                # Build sixel value for this column
                sixel_value = 0
                for bit in range(6):
                    y = strip_y + bit
                    if y < height:
                        pixel_idx = y * width + x
                        pixel_color = pixels[pixel_idx]
                        if color_to_index[pixel_color] == color_idx:
                            sixel_value |= (1 << bit)

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

            # Flush last run for this color plane
            flush_run()

            sixel_data.append("$")  # Carriage return

    # End Sixel sequence
    sixel_data.append("\x1b\\")  # ST (String Terminator)

    return "".join(sixel_data)


def _parse_cell_px(cell_px: str) -> tuple[int, int]:
    """Parse a WxH string into integer pixel cell dimensions.

    Defaults to (8,16) if parsing fails.
    """
    try:
        w_s, h_s = cell_px.lower().split("x", 1)
        w = int(w_s.strip())
        h = int(h_s.strip())
        if w > 0 and h > 0:
            return (w, h)
    except Exception:
        pass
    return (8, 16)


def display_image_simple(
    image_path,
    char_width: int,
    char_height: int,
    cell_px: str = "8x16",
):
    """
    Simple display using libsixel if available, otherwise use basic converter.

    Args:
        image_path: Path to the image file
        char_width: Terminal width in characters
        char_height: Terminal height in characters
    """
    # Try to use libsixel via img2sixel if available
    import subprocess

    img2sixel_path = shutil.which("img2sixel")

    if img2sixel_path:
        print(f"Using img2sixel to display: {image_path}")
        try:
            # img2sixel automatically handles conversion
            # -w width in pixels, -h height in pixels
            # Derive pixel size from character cells
            cell_w, cell_h = _parse_cell_px(cell_px)
            pixel_width = max(1, char_width * cell_w)
            pixel_height = max(1, char_height * cell_h)

            result = subprocess.run(
                [img2sixel_path, "-w", str(pixel_width), "-h", str(pixel_height), str(image_path)],
                check=True,
                capture_output=False
            )
            return
        except subprocess.CalledProcessError as e:
            print(f"img2sixel failed: {e}")
            print("Falling back to Python implementation...")

    # Fallback to Python implementation
    print(f"Converting and displaying: {image_path}")

    # Convert character dimensions to approximate pixels
    cell_w, cell_h = _parse_cell_px(cell_px)
    pixel_width = max(1, int(char_width) * cell_w)
    pixel_height = max(1, int(char_height) * cell_h)

    sixel_str = image_to_sixel(image_path, pixel_width, pixel_height)

    # Output to terminal
    sys.stdout.write(sixel_str)
    sys.stdout.flush()


def _detect_terminal_chars() -> tuple[int, int]:
    """Detect current terminal columns/rows, fallback to 80x24."""
    size = shutil.get_terminal_size(fallback=(80, 24))
    cols = int(os.environ.get("COLUMNS", size.columns))
    rows = int(os.environ.get("LINES", size.lines))
    # Leave a small margin
    cols = max(20, cols - 2)
    rows = max(10, rows - 2)
    return cols, rows


def main():
    parser = argparse.ArgumentParser(
        description="Display images in Sixel format in the terminal"
    )
    parser.add_argument("image", help="Path to image file")
    parser.add_argument(
        "-w",
        "--width",
        type=int,
        default=None,
        help="Maximum width in terminal characters (default: auto-detect)",
    )
    parser.add_argument(
        "--height",
        type=int,
        default=None,
        help="Maximum height in terminal characters (default: auto-detect)",
    )
    parser.add_argument(
        "--no-libsixel",
        action="store_true",
        help="Don't use libsixel even if available"
    )
    parser.add_argument(
        "--cell-px",
        type=str,
        default=os.environ.get("CELL_SIZE_PX", "8x16"),
        help="Character cell size in pixels as WxH (default 8x16). Override if your font uses different metrics.",
    )
    parser.add_argument(
        "--scale",
        type=float,
        default=0.05,
        help="Scale factor applied to detected or specified size (default: 0.05).",
    )

    args = parser.parse_args()

    image_path = Path(args.image)

    if not image_path.exists():
        print(f"Error: Image file not found: {image_path}")
        sys.exit(1)

    if not image_path.is_file():
        print(f"Error: Not a file: {image_path}")
        sys.exit(1)

    # Determine target size in character cells
    if args.width is None or args.height is None:
        det_w, det_h = _detect_terminal_chars()
        char_w = int(round(det_w * args.scale))
        char_h = int(round(det_h * args.scale))
    else:
        char_w = int(round(args.width * args.scale))
        char_h = int(round(args.height * args.scale))

    display_image_simple(image_path, char_w, char_h, args.cell_px)


if __name__ == "__main__":
    main()
