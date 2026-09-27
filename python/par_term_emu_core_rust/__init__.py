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

# Optional streaming support (available when built with --features streaming).
# The stub classes registered in non-streaming builds still import, so probe
# the native HAS_STREAMING constant instead of catching ImportError.
from typing import Any

from ._native import (
    HAS_STREAMING,
    AmbiguousWidth,
    Attributes,
    CoprocessConfig,
    CursorStyle,
    Graphic,
    ImageDimension,
    ImagePlacement,
    Macro,
    MacroEvent,
    MouseEncoding,
    NormalizationForm,
    ProgressBar,
    ProgressState,
    PtyTerminal,
    RecordingEvent,
    RecordingSession,
    ScreenshotConfig,
    ScreenSnapshot,
    SelectionMode,
    ShellIntegration,
    Terminal,
    Trigger,
    TriggerAction,
    TriggerMatch,
    UnderlineStyle,
    UnicodeVersion,
    WidthConfig,
    # Color utility functions
    adjust_contrast_rgb,
    adjust_hue,
    adjust_saturation,
    # Unicode width functions
    char_width,
    char_width_cjk,
    color_luminance,
    complementary_color,
    contrast_ratio,
    darken_rgb,
    hex_to_rgb,
    hsl_to_rgb,
    is_dark_color,
    is_east_asian_ambiguous,
    lighten_rgb,
    meets_wcag_aa,
    meets_wcag_aaa,
    mix_colors,
    perceived_brightness_rgb,
    rgb_to_ansi_256,
    rgb_to_hex,
    rgb_to_hsl,
    str_width,
    str_width_cjk,
)

_has_streaming = HAS_STREAMING

if _has_streaming:
    from ._native import (
        StreamingConfig,
        StreamingServer,
        decode_client_message,
        decode_server_message,
        encode_client_message,
        encode_server_message,
    )
else:
    # Static consumers keep the _native.pyi surface (streaming classes are
    # declared there unconditionally), so the None fallbacks are typed Any.
    StreamingConfig: Any = None
    StreamingServer: Any = None
    encode_server_message: Any = None
    decode_server_message: Any = None
    encode_client_message: Any = None
    decode_client_message: Any = None

from .observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)

__version__ = "0.53.0"
__all__ = [
    "AmbiguousWidth",
    "Attributes",
    "CoprocessConfig",
    "CursorStyle",
    "Graphic",
    "ImageDimension",
    "ImagePlacement",
    "Macro",
    "MacroEvent",
    "MouseEncoding",
    "NormalizationForm",
    "ProgressBar",
    "ProgressState",
    "PtyTerminal",
    "RecordingEvent",
    "RecordingSession",
    "ScreenSnapshot",
    "ScreenshotConfig",
    "SelectionMode",
    "ShellIntegration",
    "Terminal",
    "Trigger",
    "TriggerAction",
    "TriggerMatch",
    "UnderlineStyle",
    "UnicodeVersion",
    "WidthConfig",
    # Color utility functions
    "adjust_contrast_rgb",
    "adjust_hue",
    "adjust_saturation",
    # Unicode width functions
    "char_width",
    "char_width_cjk",
    "color_luminance",
    "complementary_color",
    "contrast_ratio",
    "darken_rgb",
    "hex_to_rgb",
    "hsl_to_rgb",
    "is_dark_color",
    "is_east_asian_ambiguous",
    "lighten_rgb",
    "meets_wcag_aa",
    "meets_wcag_aaa",
    "mix_colors",
    # Observer convenience wrappers
    "on_bell",
    "on_command_complete",
    "on_cwd_change",
    "on_title_change",
    "on_zone_change",
    "perceived_brightness_rgb",
    "rgb_to_ansi_256",
    "rgb_to_hex",
    "rgb_to_hsl",
    "str_width",
    "str_width_cjk",
]

# Add streaming classes and functions if available
if _has_streaming:
    __all__.extend(
        [
            "StreamingConfig",
            "StreamingServer",
            "decode_client_message",
            "decode_server_message",
            "encode_client_message",
            "encode_server_message",
        ]
    )
