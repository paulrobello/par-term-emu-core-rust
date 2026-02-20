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
    ProgressBar,
    ProgressState,
    PtyTerminal,
    RecordingEvent,
    RecordingSession,
    ScreenSnapshot,
    ShellIntegration,
    Terminal,
    Trigger,
    TriggerAction,
    TriggerMatch,
    NormalizationForm,
    UnderlineStyle,
    UnicodeVersion,
    WidthConfig,
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
    # Unicode width functions
    char_width,
    char_width_cjk,
    str_width,
    str_width_cjk,
    is_east_asian_ambiguous,
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

from .observers import (
    on_bell,
    on_command_complete,
    on_cwd_change,
    on_title_change,
    on_zone_change,
)

__version__ = "0.39.1"
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
    "ProgressBar",
    "ProgressState",
    "PtyTerminal",
    "RecordingEvent",
    "RecordingSession",
    "ScreenSnapshot",
    "ShellIntegration",
    "Terminal",
    "Trigger",
    "TriggerAction",
    "TriggerMatch",
    "NormalizationForm",
    "UnderlineStyle",
    "UnicodeVersion",
    "WidthConfig",
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
    # Unicode width functions
    "char_width",
    "char_width_cjk",
    "str_width",
    "str_width_cjk",
    "is_east_asian_ambiguous",
    # Observer convenience wrappers
    "on_bell",
    "on_command_complete",
    "on_cwd_change",
    "on_title_change",
    "on_zone_change",
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
