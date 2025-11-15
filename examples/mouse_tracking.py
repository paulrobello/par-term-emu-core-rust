#!/usr/bin/env python3
"""
Mouse tracking demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 24)

    print("=== Mouse Tracking Demo ===\n")

    # Initially mouse tracking is off
    print(f"Mouse mode: {term.mouse_mode()}")
    print(f"Focus tracking: {term.focus_tracking()}")
    print()

    # Enable normal mouse tracking (press and release)
    print("Enabling normal mouse tracking (X11 mode)...")
    term.process_str("\x1b[?1000h")
    print(f"Mouse mode: {term.mouse_mode()}")
    print()

    # Simulate a mouse click
    print("Simulating left mouse button click at (10, 5):")
    # Button 0 = left, col=10, row=5, pressed=True
    event_press = term.simulate_mouse_event(0, 10, 5, True)
    print(f"  Press sequence: {event_press!r}")

    event_release = term.simulate_mouse_event(0, 10, 5, False)
    print(f"  Release sequence: {event_release!r}")
    print()

    # Switch to SGR mouse encoding (1006)
    print("Enabling SGR mouse encoding...")
    term.process_str("\x1b[?1006h")

    print("Simulating mouse click with SGR encoding:")
    event_press = term.simulate_mouse_event(0, 10, 5, True)
    print(f"  Press sequence: {event_press!r}")

    event_release = term.simulate_mouse_event(0, 10, 5, False)
    print(f"  Release sequence: {event_release!r}")
    print()

    # Enable button-event tracking
    print("Enabling button-event tracking (motion while pressed)...")
    term.process_str("\x1b[?1002h")
    print(f"Mouse mode: {term.mouse_mode()}")
    print()

    # Simulate drag operation
    print("Simulating mouse drag:")
    print(f"  Press at (5, 3): {term.simulate_mouse_event(0, 5, 3, True)!r}")
    print(f"  Move to (10, 3): {term.simulate_mouse_event(0, 10, 3, True)!r}")
    print(f"  Move to (15, 3): {term.simulate_mouse_event(0, 15, 3, True)!r}")
    print(f"  Release at (15, 3): {term.simulate_mouse_event(0, 15, 3, False)!r}")
    print()

    # Enable any-event tracking
    print("Enabling any-event tracking (all motion)...")
    term.process_str("\x1b[?1003h")
    print(f"Mouse mode: {term.mouse_mode()}")
    print()

    # Different mouse buttons
    print("Different mouse buttons:")
    print(f"  Left (0): {term.simulate_mouse_event(0, 10, 5, True)!r}")
    print(f"  Middle (1): {term.simulate_mouse_event(1, 10, 5, True)!r}")
    print(f"  Right (2): {term.simulate_mouse_event(2, 10, 5, True)!r}")
    print()

    # Disable mouse tracking
    print("Disabling mouse tracking...")
    term.process_str("\x1b[?1000l")
    print(f"Mouse mode: {term.mouse_mode()}")
    print()

    # Focus tracking
    print("=== Focus Tracking Demo ===\n")

    print("Enabling focus tracking...")
    term.process_str("\x1b[?1004h")
    print(f"Focus tracking: {term.focus_tracking()}")
    print()

    print("Focus events:")
    print(f"  Focus in: {term.get_focus_in_event()!r}")
    print(f"  Focus out: {term.get_focus_out_event()!r}")
    print()

    # Disable focus tracking
    term.process_str("\x1b[?1004l")
    print(f"Focus tracking after disable: {term.focus_tracking()}")
    print()

    # Practical example: a clickable interface
    print("=== Practical Example: Clickable Button ===\n")

    term.reset()
    term.process_str("\x1b[?1000h\x1b[?1006h")  # Enable mouse with SGR

    # Draw a button
    term.process_str("\x1b[5;20H╔══════════╗\n")
    term.process_str("\x1b[6;20H║  \x1b[1mCLICK ME\x1b[0m ║\n")
    term.process_str("\x1b[7;20H╚══════════╝\n")

    print("Interface with clickable button:")
    print_terminal_content(term, show_colors=True)
    print()

    # Check if click is within button bounds
    button_col_range = range(20, 32)
    button_row_range = range(4, 7)  # 0-indexed

    test_clicks = [
        (25, 5, "Inside button"),
        (10, 5, "Outside button"),
        (30, 6, "Inside button"),
    ]

    print("Simulating clicks:")
    for col, row, desc in test_clicks:
        event = term.simulate_mouse_event(0, col, row, True)
        inside = (col in button_col_range) and (row in button_row_range)
        print(
            f"  Click at ({col}, {row}) - {desc}: {'✓ Accepted' if inside else '✗ Ignored'}"
        )
        print(f"    Event: {event!r}")


if __name__ == "__main__":
    main()
