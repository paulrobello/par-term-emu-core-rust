//! A comprehensive terminal emulator library in Rust with Python bindings
//!
//! This library provides full VT100/VT220/VT320/VT420/VT520 terminal emulation with iTerm2 feature parity:
//!
//! ## VT Compatibility Features
//! - **VT100**: Basic ANSI escape sequences, cursor control, colors
//! - **VT220**: Line/character editing (IL, DL, ICH, DCH, ECH)
//! - **VT320**: Extended features and modes
//! - **VT420**: Rectangle operations, character protection, left/right margins
//! - **VT520**: Conformance level control, bell volume control
//!
//! ## Color Support
//! - Basic 16 ANSI colors
//! - 256-color palette
//! - True color (24-bit RGB) support
//!
//! ## Advanced Features
//! - Scrollback buffer with configurable size
//! - Text attributes (bold, italic, underline, strikethrough, blink, reverse, dim, hidden)
//! - Comprehensive cursor control and positioning
//! - Scrolling regions (DECSTBM)
//! - Tab stops with HTS, TBC, CHT, CBT
//! - Terminal resizing
//! - Alternate screen buffer (with multiple variants)
//! - Mouse reporting (X10, Normal, Button, Any modes)
//! - Mouse encodings (Default, UTF-8, SGR, URXVT)
//! - Bracketed paste mode
//! - Focus tracking
//! - Application cursor keys mode
//! - Origin mode (DECOM)
//! - Auto wrap mode (DECAWM)
//! - Shell integration (OSC 133)
//! - OSC 8 hyperlinks (recognized)
//! - Full Unicode support including emoji and wide characters
//! - Bell event tracking for visual bell implementations

pub mod ansi_utils;
pub mod cell;
pub mod color;
pub mod color_utils;
pub mod conformance_level;
pub mod cursor;
#[macro_use]
pub mod debug;
pub mod grapheme;
pub mod graphics;
pub mod grid;
pub mod html_export;
pub mod macros;
pub mod mouse;
pub mod pty_error;
pub mod pty_session;
#[cfg(feature = "python")]
pub mod python_bindings;
pub mod screenshot;
pub mod shell_integration;
pub mod sixel;
pub mod streaming;
pub mod terminal;
pub mod text_utils;
pub mod tmux_control;

#[cfg(feature = "python")]
use pyo3::exceptions::{PyIOError, PyRuntimeError};
#[cfg(feature = "python")]
use pyo3::prelude::*;

// Re-export Python bindings for convenience
#[cfg(feature = "python")]
pub use python_bindings::{
    decode_client_message, decode_server_message, encode_client_message, encode_server_message,
    py_adjust_contrast_rgb, py_adjust_hue, py_adjust_saturation, py_color_luminance,
    py_complementary_color, py_contrast_ratio, py_darken_rgb, py_hex_to_rgb, py_hsl_to_rgb,
    py_is_dark_color, py_lighten_rgb, py_meets_wcag_aa, py_meets_wcag_aaa, py_mix_colors,
    py_perceived_brightness_rgb, py_rgb_to_ansi_256, py_rgb_to_hex, py_rgb_to_hsl, PyAttributes,
    PyBenchmarkResult, PyBenchmarkSuite, PyBookmark, PyClipboardEntry, PyClipboardHistoryEntry,
    PyClipboardSyncEvent, PyColorHSL, PyColorHSV, PyColorPalette, PyCommandExecution,
    PyComplianceReport, PyComplianceTest, PyCursorStyle, PyCwdChange, PyDamageRegion,
    PyDetectedItem, PyEscapeSequenceProfile, PyFrameTiming, PyGraphic, PyImageFormat,
    PyImageProtocol, PyInlineImage, PyJoinedLines, PyLineDiff, PyMacro, PyMacroEvent,
    PyMouseEncoding, PyMouseEvent, PyMousePosition, PyNotificationConfig, PyNotificationEvent,
    PyPaneState, PyPerformanceMetrics, PyProfilingData, PyProgressBar, PyProgressState,
    PyPtyTerminal, PyRecordingEvent, PyRecordingSession, PyRegexMatch, PyRenderingHint,
    PyScreenSnapshot, PyScrollbackStats, PySearchMatch, PySelection, PySelectionMode,
    PySessionState, PyShellIntegration, PyShellIntegrationStats, PySnapshotDiff, PyStreamingConfig,
    PyStreamingServer, PyTerminal, PyTmuxNotification, PyUnderlineStyle, PyWindowLayout,
};

/// Convert PtyError to PyErr
#[cfg(feature = "python")]
impl From<pty_error::PtyError> for PyErr {
    fn from(err: pty_error::PtyError) -> PyErr {
        match err {
            pty_error::PtyError::ProcessSpawnError(msg) => {
                PyRuntimeError::new_err(format!("Failed to spawn process: {}", msg))
            }
            pty_error::PtyError::ProcessExitedError(code) => {
                PyRuntimeError::new_err(format!("Process has exited with code: {}", code))
            }
            pty_error::PtyError::IoError(err) => PyIOError::new_err(err.to_string()),
            pty_error::PtyError::ResizeError(msg) => {
                PyRuntimeError::new_err(format!("Failed to resize PTY: {}", msg))
            }
            pty_error::PtyError::NotStartedError => {
                PyRuntimeError::new_err("PTY session has not been started")
            }
            pty_error::PtyError::LockError(msg) => {
                PyRuntimeError::new_err(format!("Mutex lock error: {}", msg))
            }
        }
    }
}

/// A comprehensive terminal emulator library
#[cfg(feature = "python")]
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Sixel rendering mode constants
    m.add("SIXEL_DISABLED", "disabled")?;
    m.add("SIXEL_PIXELS", "pixels")?;
    m.add("SIXEL_HALFBLOCKS", "halfblocks")?;

    // Classes
    m.add_class::<PyTerminal>()?;
    m.add_class::<PyPtyTerminal>()?;
    m.add_class::<PyAttributes>()?;
    m.add_class::<PyScreenSnapshot>()?;
    m.add_class::<PyShellIntegration>()?;
    m.add_class::<PyGraphic>()?;
    m.add_class::<PyTmuxNotification>()?;
    m.add_class::<PyCursorStyle>()?;
    m.add_class::<PyUnderlineStyle>()?;
    m.add_class::<PyMouseEncoding>()?;
    m.add_class::<PySearchMatch>()?;
    m.add_class::<PyDetectedItem>()?;
    m.add_class::<PySelection>()?;
    m.add_class::<PySelectionMode>()?;
    m.add_class::<PyScrollbackStats>()?;
    m.add_class::<PyBookmark>()?;
    m.add_class::<PyPerformanceMetrics>()?;
    m.add_class::<PyFrameTiming>()?;
    m.add_class::<PyColorHSV>()?;
    m.add_class::<PyColorHSL>()?;
    m.add_class::<PyColorPalette>()?;
    m.add_class::<PyJoinedLines>()?;
    m.add_class::<PyClipboardEntry>()?;
    m.add_class::<PyMouseEvent>()?;
    m.add_class::<PyMousePosition>()?;
    m.add_class::<PyDamageRegion>()?;
    m.add_class::<PyRenderingHint>()?;
    m.add_class::<PyEscapeSequenceProfile>()?;
    m.add_class::<PyProfilingData>()?;
    m.add_class::<PyLineDiff>()?;
    m.add_class::<PySnapshotDiff>()?;
    m.add_class::<PyRegexMatch>()?;
    m.add_class::<PyPaneState>()?;
    m.add_class::<PyWindowLayout>()?;
    m.add_class::<PySessionState>()?;
    m.add_class::<PyImageProtocol>()?;
    m.add_class::<PyImageFormat>()?;
    m.add_class::<PyInlineImage>()?;
    m.add_class::<PyBenchmarkResult>()?;
    m.add_class::<PyBenchmarkSuite>()?;
    m.add_class::<PyComplianceTest>()?;
    m.add_class::<PyComplianceReport>()?;
    m.add_class::<PyClipboardSyncEvent>()?;
    m.add_class::<PyClipboardHistoryEntry>()?;
    m.add_class::<PyCommandExecution>()?;
    m.add_class::<PyShellIntegrationStats>()?;
    m.add_class::<PyCwdChange>()?;
    m.add_class::<PyNotificationEvent>()?;
    m.add_class::<PyNotificationConfig>()?;
    m.add_class::<PyRecordingEvent>()?;
    m.add_class::<PyRecordingSession>()?;
    m.add_class::<PyMacro>()?;
    m.add_class::<PyMacroEvent>()?;
    m.add_class::<PyStreamingServer>()?;
    m.add_class::<PyStreamingConfig>()?;
    m.add_class::<PyProgressState>()?;
    m.add_class::<PyProgressBar>()?;

    // Color utility functions
    m.add_function(wrap_pyfunction!(py_perceived_brightness_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_adjust_contrast_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_lighten_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_darken_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_color_luminance, m)?)?;
    m.add_function(wrap_pyfunction!(py_is_dark_color, m)?)?;
    m.add_function(wrap_pyfunction!(py_contrast_ratio, m)?)?;
    m.add_function(wrap_pyfunction!(py_meets_wcag_aa, m)?)?;
    m.add_function(wrap_pyfunction!(py_meets_wcag_aaa, m)?)?;
    m.add_function(wrap_pyfunction!(py_mix_colors, m)?)?;
    m.add_function(wrap_pyfunction!(py_rgb_to_hsl, m)?)?;
    m.add_function(wrap_pyfunction!(py_hsl_to_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_adjust_saturation, m)?)?;
    m.add_function(wrap_pyfunction!(py_adjust_hue, m)?)?;
    m.add_function(wrap_pyfunction!(py_complementary_color, m)?)?;
    m.add_function(wrap_pyfunction!(py_rgb_to_hex, m)?)?;
    m.add_function(wrap_pyfunction!(py_hex_to_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(py_rgb_to_ansi_256, m)?)?;

    // Binary protocol functions for streaming
    m.add_function(wrap_pyfunction!(encode_server_message, m)?)?;
    m.add_function(wrap_pyfunction!(decode_server_message, m)?)?;
    m.add_function(wrap_pyfunction!(encode_client_message, m)?)?;
    m.add_function(wrap_pyfunction!(decode_client_message, m)?)?;

    Ok(())
}
