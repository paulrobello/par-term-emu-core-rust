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
    Macro,
    MacroEvent,
    MouseEncoding,
    ProgressBar,
    ProgressState,
    PtyTerminal,
    ScreenSnapshot,
    ShellIntegration,
    Terminal,
    UnderlineStyle,
    # Color utility functions
    adjust_contrast_rgb,
    adjust_hue,
    adjust_saturation,
    color_luminance,
    complementary_color,
    contrast_ratio,
    darken_rgb,
    hex_to_rgb,
    hsl_to_rgb,
    is_dark_color,
    lighten_rgb,
    meets_wcag_aa,
    meets_wcag_aaa,
    mix_colors,
    perceived_brightness_rgb,
    rgb_to_ansi_256,
    rgb_to_hex,
    rgb_to_hsl,
)

# Optional streaming support (available when built with --features streaming)
try:
    from ._native import (
        StreamingConfig,
        StreamingServer,
        encode_server_message,
        decode_server_message,
        encode_client_message,
        decode_client_message,
    )

    _has_streaming = True
except ImportError:
    _has_streaming = False
    StreamingConfig = None
    StreamingServer = None
    encode_server_message = None
    decode_server_message = None
    encode_client_message = None
    decode_client_message = None

__version__ = "0.23.0"
__all__ = [
    "Attributes",
    "CursorStyle",
    "Graphic",
    "Macro",
    "MacroEvent",
    "MouseEncoding",
    "ProgressBar",
    "ProgressState",
    "PtyTerminal",
    "ScreenSnapshot",
    "ShellIntegration",
    "Terminal",
    "UnderlineStyle",
    # Color utility functions
    "adjust_contrast_rgb",
    "adjust_hue",
    "adjust_saturation",
    "color_luminance",
    "complementary_color",
    "contrast_ratio",
    "darken_rgb",
    "hex_to_rgb",
    "hsl_to_rgb",
    "is_dark_color",
    "lighten_rgb",
    "meets_wcag_aa",
    "meets_wcag_aaa",
    "mix_colors",
    "perceived_brightness_rgb",
    "rgb_to_ansi_256",
    "rgb_to_hex",
    "rgb_to_hsl",
]

# Add streaming classes and functions if available
if _has_streaming:
    __all__.extend(
        [
            "StreamingConfig",
            "StreamingServer",
            "encode_server_message",
            "decode_server_message",
            "encode_client_message",
            "decode_client_message",
        ]
    )
