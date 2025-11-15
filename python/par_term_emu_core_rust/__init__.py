"""
par_term_emu - A comprehensive terminal emulator library

This library provides a full-featured terminal emulator with support for:
- ANSI/VT100 escape sequences
- True color (24-bit RGB) support
- 256-color palette
- Scrollback buffer
- Text attributes (bold, italic, underline, etc.)
- Terminal resizing
- Alternate screen buffer
- Mouse reporting (multiple protocols)
- Bracketed paste mode
- Focus tracking
- Shell integration (OSC 133)
- Full Unicode support including emoji and wide characters
- PTY support for running shell processes (PtyTerminal)
"""

from ._native import (
    Attributes,
    CursorStyle,
    Graphic,
    PtyTerminal,
    ScreenSnapshot,
    ShellIntegration,
    Terminal,
    UnderlineStyle,
)

__version__ = "0.2.0"
__all__ = [
    "Attributes",
    "CursorStyle",
    "Graphic",
    "PtyTerminal",
    "ScreenSnapshot",
    "ShellIntegration",
    "Terminal",
    "UnderlineStyle",
]
