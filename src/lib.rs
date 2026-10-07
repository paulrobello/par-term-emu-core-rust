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
//!
//! ## API tiers (Rust embedders)
//! - **Core**: [`prelude`] re-exports the types nearly every embedder uses
//!   (`Terminal`, `Grid`, `Cell`, `Color`, `Cursor`, events and observers,
//!   `TerminalGraphic`, `TmuxNotification`, width configuration, and
//!   `PtySession` with `pty_session`). Start with
//!   `use par_term_emu_core_rust::prelude::*;`.
//! - **Peripheral**: everything else is reached through its module path
//!   (`terminal::…`, `graphics::…`, `badge::…`, `streaming::…`, `mux::…`);
//!   the [`prelude`] docs list the canonical path per area.
//! - **Legacy crate-root re-exports**: the root-level `pub use` lines below
//!   predate the tiering and stay for existing consumers. The PyO3 wrapper
//!   re-exports are hidden from the docs; their canonical path is
//!   `python_bindings::…`.

// QA-201: every production `unsafe` block states its invariant. Test modules
// are exempt — their FFI calls restate the fn contract and add only noise.
#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]
// DOC-128: every public item is documented; `make lint-check` fails on a new
// undocumented one (clippy runs with -D warnings).
//
// `missing_docs` allow-list (scaffolding to shrink; never add a crate-wide
// allow). Every `#[allow(missing_docs)]` in the tree must be listed here.
//
//   src/streaming/proto.rs  `pb` module   ~217 hits (2026-09-30 count)
//       Generated prost output (`terminal.pb.rs`); regeneration would discard
//       hand-written docs. The wire contract lives in proto/terminal.proto.
//       Next slice: emit docs from the .proto comments, then drop the allow.
#![warn(missing_docs)]

// ARC-007 O2: the terminal core lives in the `par-term-emu-core` workspace
// member (crates/par-term-emu-core, see its DESIGN.md). Every module keeps its
// historical `par_term_emu_core_rust::<module>` path through these re-exports.
#[cfg(feature = "pty_session")]
pub use par_term_emu_core::pty_session;
#[cfg(feature = "screenshot")]
pub use par_term_emu_core::screenshot;
pub use par_term_emu_core::{
    ansi_utils, badge, cell, color, color_utils, conformance_level, coprocess, cursor, debug,
    grapheme, graphics, grid, html_export, keyboard, macros, mouse, pty_error, shell_integration,
    sixel, terminal, text_utils, tmux_control, unicode_normalization_config, unicode_width_config,
    zone,
};

// `#[macro_export]` debug macros: `crate::debug_log!` (root) and
// `par_term_emu_core_rust::debug_log!` (embedders) keep resolving.
pub use par_term_emu_core::{debug_error, debug_info, debug_log, debug_trace};

// `sim` is an empty marker feature naming the headless profile; it is meant
// to be used alone (`default-features = false, features = ["sim"]`). Combined
// with the default `python` feature it selects nothing while still compiling
// the full PyO3 surface, so make that misuse loud instead of silently
// producing the full build.
#[cfg(all(feature = "sim", feature = "python"))]
compile_error!(
    "`sim` is the headless profile and must be built with --no-default-features \
     (e.g. `cargo build --no-default-features --features sim`); combining it with \
     the `python` feature selects nothing"
);
// The C ABI (`ptec_terminal_*` exports) is opt-in so Python wheels and Rust
// embedders do not export unprefixed global symbols (ARC-112); the
// xcframework build and C/Swift embedders enable `ffi`.
#[cfg(feature = "ffi")]
pub mod ffi;
// Exercises `ffi::SharedState` against a live Terminal; lived under
// terminal/tests/ before the core moved to its own crate.
#[cfg(all(test, feature = "ffi"))]
mod ffi_tests;
// ARC-007 O2 Phase 2: the multiplexer lives in the `par-mux` workspace member
// (crates/par-mux); `par_term_emu_core_rust::mux::…` keeps resolving.
#[cfg(feature = "mux")]
pub use par_mux::mux;
pub mod prelude;
#[cfg(any(feature = "python", feature = "python-test"))]
pub mod python_bindings;
// The streaming module compiles for the streaming server itself and for the
// Python bindings (whose codec entry points stub out when `streaming` is
// off); headless profiles (sim/rust-only/mux) pull neither it nor its deps.
#[cfg(any(feature = "streaming", feature = "python", feature = "python-test"))]
pub mod streaming;

// The observer module lives in the terminal layer (its types are Terminal
// state and dispatch); the crate-root path `crate::observer` is kept as a
// re-export so existing internal and external paths still resolve (ARC-108).
pub use terminal::observer;

// Re-export commonly used types from unicode_normalization_config
pub use unicode_normalization_config::NormalizationForm;

// Re-export commonly used types from unicode_width_config
pub use unicode_width_config::{
    char_width, char_width_cjk, is_east_asian_ambiguous, str_width, str_width_cjk, AmbiguousWidth,
    UnicodeVersion, WidthConfig,
};

// Re-export recording types for session logging/recording
pub use terminal::{
    RecordingEvent, RecordingEventType, RecordingExportFormat, RecordingFormat, RecordingSession,
};

// Re-export badge types for badge format support
pub use badge::{
    decode_badge_format, evaluate_badge_format, BadgeFormatChanged, BadgeFormatError,
    SessionVariables,
};

#[cfg(any(feature = "python", feature = "python-test"))]
use pyo3::exceptions::{PyIOError, PyRuntimeError};
#[cfg(any(feature = "python", feature = "python-test"))]
use pyo3::prelude::*;

// Legacy convenience re-exports of the PyO3 wrapper types (ARC-005). The
// canonical path is `python_bindings::…`; `register_classes` and friends below
// use these names unqualified. Hidden from rustdoc rather than removed:
// `#[deprecated]` has no effect on `use` items.
#[cfg(any(feature = "python", feature = "python-test"))]
#[doc(hidden)]
pub use python_bindings::{
    decode_client_message, decode_server_message, encode_client_message, encode_server_message,
    py_adjust_contrast_rgb, py_adjust_hue, py_adjust_saturation, py_char_width, py_char_width_cjk,
    py_color_luminance, py_complementary_color, py_contrast_ratio, py_darken_rgb, py_hex_to_rgb,
    py_hsl_to_rgb, py_is_dark_color, py_is_east_asian_ambiguous, py_lighten_rgb, py_meets_wcag_aa,
    py_meets_wcag_aaa, py_mix_colors, py_perceived_brightness_rgb, py_rgb_to_ansi_256,
    py_rgb_to_hex, py_rgb_to_hsl, py_str_width, py_str_width_cjk, PyAmbiguousWidth, PyAttributes,
    PyBenchmarkResult, PyBenchmarkSuite, PyBookmark, PyClipboardEntry, PyClipboardHistoryEntry,
    PyClipboardSyncEvent, PyColorHSL, PyColorHSV, PyColorPalette, PyCommandExecution,
    PyComplianceReport, PyComplianceTest, PyCoprocessConfig, PyCursorStyle, PyCwdChange,
    PyDamageRegion, PyDetectedItem, PyEscapeSequenceProfile, PyFrameTiming, PyGraphic,
    PyImageDimension, PyImageFormat, PyImagePlacement, PyImageProtocol, PyInlineImage,
    PyJoinedLines, PyLineDiff, PyMacro, PyMacroEvent, PyMouseEncoding, PyMouseEvent,
    PyMousePosition, PyNormalizationForm, PyNotification, PyNotificationConfig,
    PyNotificationEvent, PyPerformanceMetrics, PyProfilingData, PyProgressBar, PyProgressState,
    PyPtyTerminal, PyRecordingEvent, PyRecordingSession, PyRegexMatch, PyRenderingHint,
    PyScreenSnapshot, PyScreenshotConfig, PyScrollbackStats, PySearchMatch, PySelection,
    PySelectionMode, PyShellIntegration, PyShellIntegrationStats, PySnapshotDiff,
    PyStreamingConfig, PyStreamingServer, PyTerminal, PyTmuxNotification, PyTrigger,
    PyTriggerAction, PyTriggerMatch, PyUnderlineStyle, PyUnicodeVersion, PyWidthConfig,
};

// `From<PtyError | ScreenshotError | GraphicsError> for PyErr` live in
// par-term-emu-core (feature `python`): the orphan rule forbids implementing
// a foreign trait for a foreign type here.

/// Convert StreamingError to PyErr (QA-009)
#[cfg(all(feature = "python", feature = "streaming"))]
impl From<streaming::StreamingError> for PyErr {
    fn from(err: streaming::StreamingError) -> PyErr {
        use streaming::StreamingError;
        match err {
            StreamingError::IoError(e) => PyIOError::new_err(e.to_string()),
            other => PyRuntimeError::new_err(other.to_string()),
        }
    }
}

/// A comprehensive terminal emulator library
#[cfg(any(feature = "python", feature = "python-test"))]
#[pymodule(gil_used = true)]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_constants(m)?;
    register_classes(m)?;
    register_color_utils(m)?;
    register_unicode_width(m)?;
    register_streaming_codec(m)?;

    Ok(())
}

/// Sixel rendering mode constants and other module-level scalars.
#[cfg(any(feature = "python", feature = "python-test"))]
fn register_constants(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Sixel rendering mode constants
    m.add("SIXEL_DISABLED", "disabled")?;
    m.add("SIXEL_PIXELS", "pixels")?;
    m.add("SIXEL_HALFBLOCKS", "halfblocks")?;

    // Build-capability flag: true when compiled with the `streaming` feature.
    m.add("HAS_STREAMING", cfg!(feature = "streaming"))?;

    Ok(())
}

/// Python classes exposed by the module.
#[cfg(any(feature = "python", feature = "python-test"))]
fn register_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Classes
    m.add_class::<PyTerminal>()?;
    m.add_class::<PyPtyTerminal>()?;
    m.add_class::<PyScreenshotConfig>()?;
    m.add_class::<PyAttributes>()?;
    m.add_class::<PyScreenSnapshot>()?;
    m.add_class::<PyShellIntegration>()?;
    m.add_class::<PyGraphic>()?;
    m.add_class::<PyImagePlacement>()?;
    m.add_class::<PyImageDimension>()?;
    m.add_class::<PyTmuxNotification>()?;
    m.add_class::<PyNotification>()?;
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
    m.add_class::<PyUnicodeVersion>()?;
    m.add_class::<PyAmbiguousWidth>()?;
    m.add_class::<PyWidthConfig>()?;
    m.add_class::<PyNormalizationForm>()?;
    m.add_class::<PyTrigger>()?;
    m.add_class::<PyTriggerMatch>()?;
    m.add_class::<PyTriggerAction>()?;
    m.add_class::<PyCoprocessConfig>()?;

    Ok(())
}

/// Color utility functions.
#[cfg(any(feature = "python", feature = "python-test"))]
fn register_color_utils(m: &Bound<'_, PyModule>) -> PyResult<()> {
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

    Ok(())
}

/// Unicode width functions.
#[cfg(any(feature = "python", feature = "python-test"))]
fn register_unicode_width(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Unicode width functions
    m.add_function(wrap_pyfunction!(py_char_width, m)?)?;
    m.add_function(wrap_pyfunction!(py_char_width_cjk, m)?)?;
    m.add_function(wrap_pyfunction!(py_str_width, m)?)?;
    m.add_function(wrap_pyfunction!(py_str_width_cjk, m)?)?;
    m.add_function(wrap_pyfunction!(py_is_east_asian_ambiguous, m)?)?;

    Ok(())
}

/// Binary protocol functions for streaming.
#[cfg(any(feature = "python", feature = "python-test"))]
fn register_streaming_codec(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Binary protocol functions for streaming
    m.add_function(wrap_pyfunction!(encode_server_message, m)?)?;
    m.add_function(wrap_pyfunction!(decode_server_message, m)?)?;
    m.add_function(wrap_pyfunction!(encode_client_message, m)?)?;
    m.add_function(wrap_pyfunction!(decode_client_message, m)?)?;

    Ok(())
}
