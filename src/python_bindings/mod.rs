//! Python bindings module
//!
//! This module contains all Python-facing bindings organized into submodules:
//! - `terminal`: PyTerminal struct and its implementation
//! - `pty`: PyPtyTerminal struct and its implementation (PTY support)
//! - `types`: Data types (PyAttributes, PyScreenSnapshot, PyShellIntegration, PyGraphic)
//! - `enums`: Enum types (PyCursorStyle, PyUnderlineStyle)
//! - `conversions`: Type conversions and parsing utilities
//! - `color_utils`: Color utility functions for contrast adjustment

pub mod color_utils;
pub mod conversions;
pub mod enums;
pub mod pty;
pub mod terminal;
pub mod types;

// Re-export public types for convenience
pub use color_utils::{
    py_adjust_contrast_rgb, py_adjust_hue, py_adjust_saturation, py_color_luminance,
    py_complementary_color, py_contrast_ratio, py_darken_rgb, py_hex_to_rgb, py_hsl_to_rgb,
    py_is_dark_color, py_lighten_rgb, py_meets_wcag_aa, py_meets_wcag_aaa, py_mix_colors,
    py_perceived_brightness_rgb, py_rgb_to_ansi_256, py_rgb_to_hex, py_rgb_to_hsl,
};
pub use enums::{PyCursorStyle, PyUnderlineStyle};
pub use pty::PyPtyTerminal;
pub use terminal::PyTerminal;
pub use types::{
    PyAttributes, PyBookmark, PyClipboardEntry, PyColorHSL, PyColorHSV, PyColorPalette,
    PyDamageRegion, PyDetectedItem, PyEscapeSequenceProfile, PyFrameTiming, PyGraphic,
    PyJoinedLines, PyLineDiff, PyMouseEvent, PyMousePosition, PyPerformanceMetrics,
    PyProfilingData, PyRenderingHint, PyScreenSnapshot, PySearchMatch, PySelection,
    PySelectionMode, PyShellIntegration, PyScrollbackStats, PySnapshotDiff, PyTmuxNotification,
};
