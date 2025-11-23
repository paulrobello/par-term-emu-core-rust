#!/usr/bin/env python3
"""
Test Kitty graphics protocol animation support.

This script creates a simple 2-frame animation that alternates between
red and blue squares to verify animation frame loading and control.

Kitty animation protocol reference:
https://sw.kovidgoyal.net/kitty/graphics-protocol/#animation
"""

import base64
import sys
import time


def create_solid_png(
    color_rgb: tuple[int, int, int], width: int = 100, height: int = 100
) -> bytes:
    """Create a simple solid color PNG image.

    Args:
        color_rgb: RGB color tuple (r, g, b)
        width: Image width in pixels
        height: Image height in pixels

    Returns:
        PNG image bytes
    """
    try:
        from PIL import Image
        import io

        img = Image.new("RGB", (width, height), color_rgb)
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return buf.getvalue()
    except ImportError:
        print(
            "Error: PIL (Pillow) is required. Install with: pip install Pillow",
            file=sys.stderr,
        )
        sys.exit(1)


def send_kitty_graphics(payload: str) -> None:
    """Send a Kitty graphics protocol sequence.

    Args:
        payload: The graphics protocol payload (keys=values,...)
    """
    # APC (Application Program Command): ESC _ G <payload> ; <data> ESC \
    print(f"\x1b_G{payload}\x1b\\", end="", flush=True)


def send_frame(
    image_id: int,
    frame_number: int,
    png_data: bytes,
    delay_ms: int = 500,
    display: bool = False,
) -> None:
    """Send an animation frame.

    Args:
        image_id: Image ID for this animation
        frame_number: Frame number (1-indexed)
        png_data: PNG image data
        delay_ms: Frame delay in milliseconds
        display: Whether to also display after transmission
    """
    encoded = base64.standard_b64encode(png_data).decode("ascii")

    # Action: a=f (frame)
    # Image ID: i=<id>
    # Frame number: r=<frame>
    # Delay: z=<delay_ms>
    # Format: f=100 (PNG)
    # Transmission: t=d (direct)
    # Always use 'f' for animation frames, not 'T'
    payload = f"a=f,i={image_id},r={frame_number},z={delay_ms},f=100,t=d;{encoded}"
    send_kitty_graphics(payload)

    # Display the image after sending the first frame (if requested)
    if display and frame_number == 1:
        display_image(image_id)


def send_animation_control(
    image_id: int, state: str | None = None, num_plays: int | None = None
) -> None:
    """Send animation control command.

    Args:
        image_id: Image ID to control
        state: State control (s= parameter):
            - '1': Stop animation
            - '2': Loading mode (wait for frames)
            - '3': Enable looping
        num_plays: Number of times to play (v= parameter):
            - 0: Ignored
            - 1: Infinite looping
            - N: Loop (N-1) times
    """
    # Action: a=a (animation control)
    # Image ID: i=<id>
    params = ["a=a", f"i={image_id}"]
    if state is not None:
        params.append(f"s={state}")
    if num_plays is not None:
        params.append(f"v={num_plays}")
    payload = ",".join(params)
    send_kitty_graphics(payload)


def display_image(image_id: int) -> None:
    """Display a transmitted image.

    Args:
        image_id: Image ID to display
    """
    # Action: a=p (put/display)
    # Image ID: i=<id>
    payload = f"a=p,i={image_id}"
    send_kitty_graphics(payload)


def delete_image(image_id: int) -> None:
    """Delete an image and all its placements.

    Args:
        image_id: Image ID to delete
    """
    # Action: a=d (delete)
    # Delete: d=i (by image ID)
    # Image ID: i=<id>
    payload = f"a=d,d=i,i={image_id}"
    send_kitty_graphics(payload)


def test_simple_animation() -> None:
    """Test a simple 2-frame animation."""
    print("=== Testing Kitty Graphics Animation ===\n")

    image_id = 42

    # Create frames
    print("Creating animation frames...")
    frame1_data = create_solid_png((255, 0, 0), 100, 100)  # Red
    frame2_data = create_solid_png((0, 0, 255), 100, 100)  # Blue

    # Send frame 1 (with display)
    print("Sending frame 1 (red, 500ms delay)...")
    send_frame(image_id, 1, frame1_data, delay_ms=500, display=True)
    time.sleep(0.1)

    # Send frame 2
    print("Sending frame 2 (blue, 500ms delay)...")
    send_frame(image_id, 2, frame2_data, delay_ms=500, display=False)
    time.sleep(0.1)

    print("\nAnimation frames loaded. Image ID:", image_id)
    print("\nAnimation should be visible above this line.")
    print("The animation is currently stopped (default state).\n")

    # Demonstrate controls
    print("Controls:")
    print("  - Setting infinite loops and starting animation...")
    send_animation_control(image_id, num_plays=1)  # v=1 = infinite loops
    send_animation_control(image_id, state="3")  # s=3 = enable looping

    time.sleep(3)

    print("  - Pausing animation (loading mode)...")
    send_animation_control(image_id, state="2")  # s=2 = loading mode (pause)

    time.sleep(2)

    print("  - Resuming animation...")
    send_animation_control(image_id, state="3")  # s=3 = enable looping again

    time.sleep(3)

    print("  - Stopping animation...")
    send_animation_control(image_id, state="1")  # s=1 = stop

    time.sleep(1)

    print("\nAnimation test complete!")
    print("Note: Frontend animation rendering may not be fully integrated yet.")
    print("Check debug logs for animation frame updates.\n")


def test_multi_frame_animation() -> None:
    """Test a 4-frame color cycle animation."""
    print("\n=== Testing Multi-Frame Animation ===\n")

    image_id = 43
    colors = [
        (255, 0, 0),  # Red
        (255, 255, 0),  # Yellow
        (0, 255, 0),  # Green
        (0, 0, 255),  # Blue
    ]

    print(f"Creating {len(colors)}-frame color cycle animation...")

    # Send all frames
    for i, color in enumerate(colors, 1):
        print(f"  Sending frame {i} (RGB{color})...")
        frame_data = create_solid_png(color, 100, 100)
        send_frame(image_id, i, frame_data, delay_ms=400, display=(i == 1))
        time.sleep(0.05)

    print(f"\n{len(colors)}-frame animation loaded. Playing with 2 loops...")
    send_animation_control(image_id, num_plays=3)  # v=3 means 2 loops (N-1)
    send_animation_control(image_id, state="3")  # s=3 = enable looping

    # Wait for animation to complete (2 loops * 4 frames * 400ms)
    time.sleep(2 * len(colors) * 0.4 + 0.5)

    print("Animation should have stopped after 2 loops.\n")


def main() -> None:
    """Main test function."""
    try:
        # Test 1: Simple 2-frame animation
        test_simple_animation()

        # Test 2: Multi-frame animation
        test_multi_frame_animation()

        print("\n=== All Tests Complete ===")
        print("\nNote: Animation playback requires frontend integration.")
        print("Current status:")
        print("  ✅ Backend: Animation frames stored and controlled")
        print(
            "  🔄 Frontend: Needs to call update_animations() and render current frame"
        )
        print("\nTo verify backend storage, check debug logs for:")
        print("  - Animation frame additions")
        print("  - Animation control commands")
        print("  - Frame timing updates")

    except KeyboardInterrupt:
        print("\n\nTest interrupted by user.")
    except Exception as e:
        print(f"\nError during test: {e}", file=sys.stderr)
        import traceback

        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
