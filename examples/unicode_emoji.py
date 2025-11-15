#!/usr/bin/env python3
"""
Unicode and emoji support demonstration
"""

from par_term_emu_core_rust import Terminal
from render_utils import print_terminal_content


def main():
    # Create a terminal
    term = Terminal(80, 30)

    print("=== Unicode and Emoji Support Demo ===\n")

    # Basic emoji
    term.process_str("Emoji: 😀 😃 😄 😁 😆 😅 🤣 😂\n")
    term.process_str("Hearts: ❤️ 💛 💚 💙 💜 🖤 🤍 🤎\n")
    term.process_str("Symbols: ✨ ⭐ 🌟 💫 ⚡ 🔥 💥 💯\n\n")

    # CJK characters (wide characters)
    term.process_str("Chinese: 你好世界 (Hello World)\n")
    term.process_str("Japanese: こんにちは世界 (Konnichiwa sekai)\n")
    term.process_str("Korean: 안녕하세요 세계 (Annyeonghaseyo segye)\n\n")

    # Mixed width characters
    term.process_str("Mixed: Hello 世界 🌍 World 🌎\n\n")

    # Box drawing characters
    term.process_str("Box drawing:\n")
    term.process_str("┌─────────────┐\n")
    term.process_str("│ Hello! 你好! │\n")
    term.process_str("└─────────────┘\n\n")

    # Math symbols
    term.process_str("Math: ∀ ∃ ∅ ∞ ∑ ∏ √ ∫ ≈ ≠ ≤ ≥\n\n")

    # Arrows
    term.process_str("Arrows: ← → ↑ ↓ ↔ ↕ ⇐ ⇒ ⇑ ⇓ ⇔\n\n")

    # Currency symbols
    term.process_str("Currency: $ € £ ¥ ₹ ₽ ₩ ₪ ₴ ₿\n\n")

    # Combining characters and diacritics
    term.process_str("Diacritics: café naïve résumé\n")
    term.process_str("Accents: àáâãäå èéêë ìíîï òóôõö ùúûü\n\n")

    # Emoji with skin tone modifiers
    term.process_str("Skin tones: 👋 👋🏻 👋🏼 👋🏽 👋🏾 👋🏿\n\n")

    # Flags
    term.process_str("Flags: 🇺🇸 🇬🇧 🇨🇦 🇦🇺 🇯🇵 🇨🇳 🇩🇪 🇫🇷\n\n")

    # Colored emoji with ANSI colors
    term.process_str("\x1b[31mRed: 🔴 ❤️ 🌹\x1b[0m\n")
    term.process_str("\x1b[32mGreen: 🟢 💚 🌿\x1b[0m\n")
    term.process_str("\x1b[34mBlue: 🔵 💙 🌊\x1b[0m\n\n")

    # True color with emoji
    term.process_str("\x1b[38;2;255;0;128mPink emoji: 💖 🌸 🦄\x1b[0m\n\n")

    # Print the content
    print_terminal_content(term, show_colors=True)

    # Demonstrate character width detection
    print("\n=== Character Width Analysis ===")
    test_chars = [
        ("A", "ASCII"),
        ("世", "CJK"),
        ("😀", "Emoji"),
        ("é", "Accented"),
    ]

    for char, desc in test_chars:
        term.reset()
        term.process_str(char)
        # The character is at position 0
        # Check next position to see if it was advanced correctly
        col, _ = term.cursor_position()
        width = col
        print(f"{desc} '{char}': width={width} column(s)")


if __name__ == "__main__":
    main()
