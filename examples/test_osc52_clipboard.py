#!/usr/bin/env python3
"""
Test OSC 52 clipboard operations.

Demonstrates the clipboard API for terminal applications that use OSC 52
sequences to read/write clipboard over SSH or other remote connections.
"""

import base64
from par_term_emu_core_rust import Terminal


def test_clipboard_write():
    """Test writing to clipboard via OSC 52."""
    print("=" * 60)
    print("OSC 52 Clipboard Write Test")
    print("=" * 60)

    term = Terminal(80, 24)

    # Initially empty
    assert term.clipboard() is None
    print("✓ Clipboard initially empty")

    # Write using OSC 52 sequence
    text = "Hello from OSC 52!"
    encoded = base64.b64encode(text.encode()).decode()
    osc52_write = f"\x1b]52;c;{encoded}\x1b\\"

    term.process_str(osc52_write)

    clipboard_content = term.clipboard()
    assert clipboard_content == text
    print(f"✓ Wrote '{text}' to clipboard")
    print(f"✓ Read back: '{clipboard_content}'")

    return True


def test_clipboard_query():
    """Test querying clipboard via OSC 52."""
    print("\n" + "=" * 60)
    print("OSC 52 Clipboard Query Test")
    print("=" * 60)

    term = Terminal(80, 24)

    # Set clipboard content
    content = "Query test content"
    term.set_clipboard(content)
    print(f"✓ Set clipboard to: '{content}'")

    # Try to query without permission (should be blocked)
    term.process_str("\x1b]52;c;?\x1b\\")
    response = term.drain_responses()
    assert response == b""
    print("✓ Query blocked when clipboard_read disabled (security)")

    # Enable clipboard read
    term.set_allow_clipboard_read(True)
    assert term.allow_clipboard_read()
    print("✓ Enabled clipboard read permission")

    # Query should now work
    term.process_str("\x1b]52;c;?\x1b\\")
    response = term.drain_responses().decode()

    # Parse response: OSC 52 ; c ; <base64> ST
    assert response.startswith("\x1b]52;c;")
    assert response.endswith("\x1b\\")

    # Extract and decode base64 data
    encoded_data = response[len("\x1b]52;c;"):-len("\x1b\\")]
    decoded = base64.b64decode(encoded_data).decode()

    assert decoded == content
    print(f"✓ Query returned: '{decoded}'")

    return True


def test_clipboard_clear():
    """Test clearing clipboard."""
    print("\n" + "=" * 60)
    print("OSC 52 Clipboard Clear Test")
    print("=" * 60)

    term = Terminal(80, 24)

    # Set some content
    term.set_clipboard("Some content")
    assert term.clipboard() == "Some content"
    print("✓ Set clipboard content")

    # Clear via OSC 52 (empty data)
    term.process_str("\x1b]52;c;\x1b\\")
    assert term.clipboard() is None
    print("✓ Cleared clipboard via OSC 52")

    # Set again
    term.set_clipboard("More content")
    assert term.clipboard() == "More content"

    # Clear via API
    term.set_clipboard(None)
    assert term.clipboard() is None
    print("✓ Cleared clipboard via API")

    return True


def test_programmatic_api():
    """Test programmatic clipboard API."""
    print("\n" + "=" * 60)
    print("Programmatic Clipboard API Test")
    print("=" * 60)

    term = Terminal(80, 24)

    # Set via API
    term.set_clipboard("API content")
    assert term.clipboard() == "API content"
    print("✓ Set clipboard via API")

    # Read via API
    content = term.clipboard()
    assert content == "API content"
    print(f"✓ Read clipboard via API: '{content}'")

    # Security flag
    assert not term.allow_clipboard_read()
    term.set_allow_clipboard_read(True)
    assert term.allow_clipboard_read()
    term.set_allow_clipboard_read(False)
    assert not term.allow_clipboard_read()
    print("✓ Security flag toggle works")

    return True


def test_ssh_workflow():
    """Simulate SSH clipboard workflow."""
    print("\n" + "=" * 60)
    print("SSH Clipboard Workflow Simulation")
    print("=" * 60)

    term = Terminal(80, 24)

    # Simulate remote app copying to clipboard
    remote_text = "Copied on remote server"
    encoded = base64.b64encode(remote_text.encode()).decode()
    osc52 = f"\x1b]52;c;{encoded}\x1b\\"

    print(f"Remote app sends OSC 52: {osc52!r}")
    term.process_str(osc52)

    # Local terminal can now access the clipboard
    local_clipboard = term.clipboard()
    assert local_clipboard == remote_text
    print(f"✓ Local clipboard updated: '{local_clipboard}'")
    print("✓ SSH clipboard integration working!")

    return True


if __name__ == "__main__":
    print("\n" + "=" * 70)
    print(" " * 20 + "OSC 52 CLIPBOARD TEST SUITE")
    print("=" * 70)

    results = []
    results.append(("Clipboard Write", test_clipboard_write()))
    results.append(("Clipboard Query", test_clipboard_query()))
    results.append(("Clipboard Clear", test_clipboard_clear()))
    results.append(("Programmatic API", test_programmatic_api()))
    results.append(("SSH Workflow", test_ssh_workflow()))

    print("\n" + "=" * 70)
    print("TEST RESULTS SUMMARY")
    print("=" * 70)

    all_passed = True
    for test_name, passed in results:
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"{status}: {test_name}")
        if not passed:
            all_passed = False

    print("=" * 70)
    if all_passed:
        print("✓ All tests passed! OSC 52 clipboard working correctly.")
        print("\nKey Features:")
        print("  - Write to clipboard via OSC 52 sequences")
        print("  - Query clipboard (with security flag)")
        print("  - Works over SSH without X11 forwarding")
        print("  - Compatible with tmux/screen")
    else:
        print("✗ Some tests failed. Please review the implementation.")

    print("=" * 70)
    exit(0 if all_passed else 1)
