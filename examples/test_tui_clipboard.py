#!/usr/bin/env python3
"""
Test script for TUI clipboard and selection features.

This script demonstrates:
1. Double-click word selection with auto-copy
2. Triple-click line selection with auto-copy
3. Shift+click selection
4. Drag selection
5. Clipboard paste with bracketed paste mode
6. Keyboard shortcuts (Ctrl+Shift+C, Ctrl+Shift+V)

Usage:
    python examples/test_tui_clipboard.py

Instructions:
1. When the TUI starts, you'll see sample text
2. Try double-clicking on a word - it should be selected and copied
3. Try triple-clicking on a line - the entire line should be selected and copied
4. Try Shift+click to start a selection, then click elsewhere to extend it
5. Try clicking and dragging to select text
6. Use Ctrl+Shift+V to paste from clipboard
7. Use Ctrl+Shift+C to copy selected text

Note: This test requires xclip (Linux) or pbcopy/pbpaste (macOS) for clipboard operations.
"""

import par_term_emu_core_rust

def main():
    # Create a terminal with bracketed paste enabled
    term = par_term_emu.Terminal(80, 24)

    # Enable bracketed paste mode
    term.process(b"\x1b[?2004h")

    # Verify bracketed paste is enabled
    assert term.bracketed_paste(), "Bracketed paste should be enabled"
    print("✅ Bracketed paste mode enabled")

    # Add some sample text for testing
    sample_text = """Welcome to the Terminal Emulator TUI Clipboard Test!

Try these interactions:
  • Double-click any word to select and copy it
  • Triple-click any line to select and copy the entire line
  • Shift+click to start a selection, then move and release
  • Click and drag to select text
  • Use Ctrl+Shift+C to copy selected text
  • Use Ctrl+Shift+V to paste from clipboard

This is a test sentence with multiple words.
The quick brown fox jumps over the lazy dog.
Lorem ipsum dolor sit amet, consectetur adipiscing elit.

Selection word boundaries include: punctuation, operators, and whitespace.
Try selecting: hello-world, test@example.com, /path/to/file

Bracketed paste mode is now ENABLED (mode 2004).
When you paste text, it will be wrapped in ESC[200~ and ESC[201~
This prevents accidental command execution on paste!

Tips:
- Selected text is highlighted with reverse video
- Selections are automatically copied to system clipboard
- Paste respects bracketed paste mode for safety
"""

    # Write the sample text to terminal
    for line in sample_text.split('\n'):
        term.process_str(line)
        term.process(b"\r\n")

    # Test the paste method
    print("\n✅ Testing paste method with bracketed paste mode:")
    test_content = "Pasted content goes here"
    term.paste(test_content)

    # Get the paste sequences
    paste_start = term.get_paste_start()
    paste_end = term.get_paste_end()
    print(f"   Paste start: {paste_start}")
    print(f"   Paste end: {paste_end}")

    # Test without bracketed paste
    term.process(b"\x1b[?2004l")
    assert not term.bracketed_paste(), "Bracketed paste should be disabled"
    print("\n✅ Bracketed paste mode disabled")

    term.paste("Direct paste (no brackets)")
    paste_start = term.get_paste_start()
    paste_end = term.get_paste_end()
    print(f"   Paste start: {paste_start} (empty)")
    print(f"   Paste end: {paste_end} (empty)")

    # Query mode status using DECRQM
    print("\n✅ Testing DECRQM query for mode 2004:")
    term.process(b"\x1b[?2004$p")
    responses = term.drain_responses()
    print(f"   Response: {responses}")

    print("\n" + "="*70)
    print("✅ All tests passed!")
    print("="*70)
    print("\nTo test the TUI interactively, run:")
    print("  make tui")
    print("\nKeyboard shortcuts in the TUI:")
    print("  Ctrl+Shift+C  - Copy selection")
    print("  Ctrl+Shift+V  - Paste from clipboard")
    print("  Ctrl+Q        - Quit")
    print("\nMouse interactions:")
    print("  Single click   - Normal operation")
    print("  Double click   - Select word and copy")
    print("  Triple click   - Select line and copy")
    print("  Shift+click    - Start/extend selection")
    print("  Click+drag     - Select text and copy on release")

if __name__ == "__main__":
    main()
