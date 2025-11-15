#!/usr/bin/env python3
"""
Comprehensive screenshot demonstration showing various features and use cases.
"""

from par_term_emu_core_rust import Terminal
import os
import sys

def demo_basic_screenshot():
    """Basic screenshot functionality"""
    print("=" * 60)
    print("Demo 1: Basic Screenshot")
    print("=" * 60)

    term = Terminal(80, 24)
    term.process_str("Hello, World!\n")
    term.process_str("\x1b[32m✓ Success\x1b[0m\n")

    try:
        # Take screenshot (will use system font)
        png_bytes = term.screenshot()
        print(f"✓ Generated PNG: {len(png_bytes):,} bytes")

        # Save to file
        term.screenshot_to_file("demo_basic.png")
        print(f"✓ Saved to: demo_basic.png")

    except Exception as e:
        print(f"✗ Error: {e}")
        print("\nIf you see a font error, please:")
        print("1. Run: ./scripts/install_font.sh")
        print("2. Or specify a font: screenshot(font_path='/path/to/font.ttf')")
        return False

    return True

def demo_colorful_output():
    """Demo with colors and styling"""
    print("\n" + "=" * 60)
    print("Demo 2: Colorful Output with Styling")
    print("=" * 60)

    term = Terminal(80, 24)

    # Title
    term.process_str("\x1b[1;36m" + "=" * 78 + "\x1b[0m\n")
    term.process_str("\x1b[1;33m  Terminal Screenshot Demo\x1b[0m\n")
    term.process_str("\x1b[1;36m" + "=" * 78 + "\x1b[0m\n\n")

    # Colored status messages
    term.process_str("\x1b[1;32m✓ Success\x1b[0m - Operation completed successfully\n")
    term.process_str("\x1b[1;33m⚠ Warning\x1b[0m - Something needs attention\n")
    term.process_str("\x1b[1;31m✗ Error\x1b[0m - Operation failed\n")
    term.process_str("\x1b[1;34mℹ Info\x1b[0m - Informational message\n\n")

    # Text attributes
    term.process_str("Text attributes: ")
    term.process_str("\x1b[1mBold\x1b[0m ")
    term.process_str("\x1b[3mItalic\x1b[0m ")
    term.process_str("\x1b[4mUnderline\x1b[0m ")
    term.process_str("\x1b[9mStrikethrough\x1b[0m\n\n")

    # Code snippet
    term.process_str("\x1b[1;35mCode Example:\x1b[0m\n")
    term.process_str("  \x1b[36mdef\x1b[0m \x1b[33mhello\x1b[0m():\n")
    term.process_str("      \x1b[32mprint\x1b[0m(\x1b[33m\"Hello, World!\"\x1b[0m)\n")

    try:
        term.screenshot_to_file("demo_colorful.png", font_size=14.0, padding=15)
        print("✓ Saved to: demo_colorful.png")
        return True
    except Exception as e:
        print(f"✗ Error: {e}")
        return False

def demo_unicode_support():
    """Demo with Unicode characters"""
    print("\n" + "=" * 60)
    print("Demo 3: Unicode Support (CJK, Emoji, Symbols)")
    print("=" * 60)

    term = Terminal(80, 24)

    term.process_str("\x1b[1;36mUnicode Character Support\x1b[0m\n\n")

    # Box drawing
    term.process_str("Box Drawing:\n")
    term.process_str("┌─────────────────────────────┐\n")
    term.process_str("│ \x1b[1mTerminal Emulator\x1b[0m         │\n")
    term.process_str("└─────────────────────────────┘\n\n")

    # Japanese
    term.process_str("Japanese: \x1b[32mこんにちは世界\x1b[0m\n")

    # Chinese
    term.process_str("Chinese:  \x1b[33m你好世界\x1b[0m\n")

    # Korean
    term.process_str("Korean:   \x1b[34m안녕하세요\x1b[0m\n\n")

    # Emoji
    term.process_str("Emoji: 🎨 🚀 ✨ 🔥 💻 🌟 ⚡ 🎯\n\n")

    # Symbols
    term.process_str("Math: ∀ ∃ ∈ ∉ ∫ ∑ ∏ √ ∞ ≈ ≠ ≤ ≥\n")
    term.process_str("Arrows: ← → ↑ ↓ ↔ ↕ ⇐ ⇒ ⇑ ⇓\n")

    try:
        term.screenshot_to_file("demo_unicode.png")
        print("✓ Saved to: demo_unicode.png")
        return True
    except Exception as e:
        print(f"✗ Error: {e}")
        return False

def demo_format_comparison():
    """Demo different image formats"""
    print("\n" + "=" * 60)
    print("Demo 4: Format Comparison (PNG, JPEG, BMP)")
    print("=" * 60)

    term = Terminal(80, 24)

    # Create content with gradients
    for i in range(20):
        color = 232 + i  # Grayscale ramp
        term.process_str(f"\x1b[48;5;{color}m \x1b[0m")
    term.process_str("\n\n")

    term.process_str("\x1b[1;36mFormat Comparison Test\x1b[0m\n")
    term.process_str("This screenshot is saved in multiple formats.\n")

    formats = [
        ("png", "PNG (lossless, best quality)"),
        ("jpg", "JPEG (lossy, smaller size)"),
        ("bmp", "BMP (uncompressed, largest size)"),
    ]

    results = {}

    for fmt, desc in formats:
        filename = f"demo_format.{fmt}"
        try:
            if fmt == "jpg":
                term.screenshot_to_file(filename, format="jpeg", quality=90)
            else:
                term.screenshot_to_file(filename, format=fmt)

            size = os.path.getsize(filename)
            results[fmt] = size
            print(f"✓ {desc:40s} {size:>10,} bytes")
        except Exception as e:
            print(f"✗ {desc:40s} Failed: {e}")

    if results:
        print("\nFile size comparison:")
        smallest = min(results.values())
        for fmt, size in results.items():
            ratio = size / smallest
            print(f"  {fmt.upper():4s}: {ratio:5.2f}x the smallest")

    return len(results) == len(formats)

def demo_custom_configuration():
    """Demo custom configuration options"""
    print("\n" + "=" * 60)
    print("Demo 5: Custom Configuration")
    print("=" * 60)

    term = Terminal(80, 24)
    term.process_str("\x1b[1;35mCustom Configuration Demo\x1b[0m\n\n")
    term.process_str("This demonstrates various configuration options:\n")
    term.process_str("• Custom font size\n")
    term.process_str("• Custom padding\n")
    term.process_str("• Quality settings (for JPEG)\n")

    configs = [
        ("demo_config_default.png", {}),
        ("demo_config_large.png", {"font_size": 18.0, "padding": 20}),
        ("demo_config_small.png", {"font_size": 10.0, "padding": 5}),
    ]

    for filename, config in configs:
        try:
            term.screenshot_to_file(filename, **config)
            desc = ", ".join(f"{k}={v}" for k, v in config.items()) if config else "default"
            print(f"✓ Saved: {filename:30s} ({desc})")
        except Exception as e:
            print(f"✗ Failed: {filename:30s} {e}")

    return True

def cleanup_demo_files():
    """Clean up demo files"""
    demo_files = [
        "demo_basic.png",
        "demo_colorful.png",
        "demo_unicode.png",
        "demo_format.png",
        "demo_format.jpg",
        "demo_format.bmp",
        "demo_config_default.png",
        "demo_config_large.png",
        "demo_config_small.png",
    ]

    removed = 0
    for filename in demo_files:
        if os.path.exists(filename):
            os.remove(filename)
            removed += 1

    if removed > 0:
        print(f"\n🗑️  Cleaned up {removed} demo files")

def main():
    """Run all demos"""
    print("\n" + "=" * 60)
    print("  Screenshot Functionality Demonstration")
    print("  par-term-emu Terminal Emulator")
    print("=" * 60)
    print()

    # Check if we should keep files
    keep_files = "--keep" in sys.argv

    demos = [
        ("Basic Screenshot", demo_basic_screenshot),
        ("Colorful Output", demo_colorful_output),
        ("Unicode Support", demo_unicode_support),
        ("Format Comparison", demo_format_comparison),
        ("Custom Configuration", demo_custom_configuration),
    ]

    results = {}
    for name, demo_func in demos:
        try:
            results[name] = demo_func()
        except Exception as e:
            print(f"\n✗ Demo '{name}' crashed: {e}")
            results[name] = False

    # Summary
    print("\n" + "=" * 60)
    print("Summary")
    print("=" * 60)

    passed = sum(1 for v in results.values() if v)
    total = len(results)

    for name, success in results.items():
        status = "✓ PASS" if success else "✗ FAIL"
        print(f"{name:30s} {status}")

    print(f"\nTotal: {passed}/{total} demos successful")

    if not keep_files:
        cleanup_demo_files()
        print("\n💡 Use --keep flag to keep demo files")
    else:
        print("\n📁 Demo files kept in current directory")

    if passed == total:
        print("\n🎉 All demos completed successfully!")
        return 0
    else:
        print(f"\n⚠️  {total - passed} demo(s) failed")
        return 1

if __name__ == "__main__":
    sys.exit(main())
