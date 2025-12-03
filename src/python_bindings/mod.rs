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
pub mod streaming;
pub mod terminal;
pub mod types;

// Re-export public types for convenience
pub use color_utils::{
    py_adjust_contrast_rgb, py_adjust_hue, py_adjust_saturation, py_color_luminance,
    py_complementary_color, py_contrast_ratio, py_darken_rgb, py_hex_to_rgb, py_hsl_to_rgb,
    py_is_dark_color, py_lighten_rgb, py_meets_wcag_aa, py_meets_wcag_aaa, py_mix_colors,
    py_perceived_brightness_rgb, py_rgb_to_ansi_256, py_rgb_to_hex, py_rgb_to_hsl,
};
pub use enums::{PyCursorStyle, PyMouseEncoding, PyProgressState, PyUnderlineStyle};
pub use pty::PyPtyTerminal;
pub use streaming::{
    decode_client_message, decode_server_message, encode_client_message, encode_server_message,
    PyStreamingConfig, PyStreamingServer,
};
pub use terminal::PyTerminal;
pub use types::{
    PyAttributes, PyBenchmarkResult, PyBenchmarkSuite, PyBookmark, PyClipboardEntry,
    PyClipboardHistoryEntry, PyClipboardSyncEvent, PyColorHSL, PyColorHSV, PyColorPalette,
    PyCommandExecution, PyComplianceReport, PyComplianceTest, PyCwdChange, PyDamageRegion,
    PyDetectedItem, PyEscapeSequenceProfile, PyFrameTiming, PyGraphic, PyImageFormat,
    PyImageProtocol, PyInlineImage, PyJoinedLines, PyLineDiff, PyMacro, PyMacroEvent, PyMouseEvent,
    PyMousePosition, PyNotificationConfig, PyNotificationEvent, PyPaneState, PyPerformanceMetrics,
    PyProfilingData, PyProgressBar, PyRecordingEvent, PyRecordingSession, PyRegexMatch,
    PyRenderingHint, PyScreenSnapshot, PyScrollbackStats, PySearchMatch, PySelection,
    PySelectionMode, PySessionState, PyShellIntegration, PyShellIntegrationStats, PySnapshotDiff,
    PyTmuxNotification, PyWindowLayout,
};
