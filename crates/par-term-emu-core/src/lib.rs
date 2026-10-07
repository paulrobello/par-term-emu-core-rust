//! Terminal emulator core for `par-term-emu-core-rust`.
//!
//! VT100/VT220/VT320/VT420/VT520 state machine ([`terminal::Terminal`]),
//! grid with scrollback ([`grid::Grid`]), Sixel/iTerm2/Kitty graphics, the
//! tmux control-mode parser, and the text/unicode utilities they share. With
//! the `pty_session` feature it adds the real PTY backend; with `screenshot`
//! the terminal-to-image renderer. Without features it is the headless
//! (`sim`) profile: no PTY, no Python, no renderer.
//!
//! Most embedders depend on `par-term-emu-core-rust`, which re-exports every
//! module here at its historical path (see `DESIGN.md`).

// QA-201: every production `unsafe` block states its invariant. Test modules
// are exempt — their FFI calls restate the fn contract and add only noise.
#![cfg_attr(not(test), warn(clippy::undocumented_unsafe_blocks))]
// DOC-128: every public item is documented; `make lint-check` fails on a new
// undocumented one (clippy runs with -D warnings).
#![warn(missing_docs)]

pub mod ansi_utils;
pub mod badge;
/// Terminal grid cells: character, colors, and attribute flags.
pub mod cell;
/// Color representation (named, 256-color palette, and true color).
pub mod color;
pub mod color_utils;
pub mod conformance_level;
pub mod coprocess;
/// Cursor position and DECSCUSR style.
pub mod cursor;
#[macro_use]
pub mod debug;
/// Grapheme cluster, variation selector, and emoji-sequence helpers.
pub mod grapheme;
pub mod graphics;
pub mod grid;
pub mod html_export;
pub mod keyboard;
pub mod macros;
/// Mouse tracking modes, encodings, and event types.
pub mod mouse;
/// Error type for PTY operations.
pub mod pty_error;
#[cfg(feature = "pty_session")]
pub mod pty_session;
// Gated so a slim sim build can drop the ~700KB embedded fonts + renderer
// (ARC-021); python/full keep it on. `sim` no longer implies it (ENH-024) —
// render-capable sim embedders add `features = ["sim", "screenshot"]`.
#[cfg(feature = "screenshot")]
pub mod screenshot;
/// Shell integration markers (OSC 133).
pub mod shell_integration;
/// Sixel graphics parsing for DEC VT340-compatible terminals.
pub mod sixel;
pub mod terminal;
pub mod text_utils;
pub mod tmux_control;
pub mod unicode_normalization_config;
pub mod unicode_width_config;
pub mod zone;

// The observer module lives in the terminal layer (its types are Terminal
// state and dispatch); the crate-root path `crate::observer` is kept as a
// re-export so existing internal and external paths still resolve (ARC-108).
pub use terminal::observer;

#[cfg(feature = "python")]
use pyo3::exceptions::{PyIOError, PyRuntimeError};
#[cfg(feature = "python")]
use pyo3::PyErr;

/// Convert PtyError to PyErr (QA-009: centralized error mapping)
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

/// Convert ScreenshotError to PyErr (QA-009)
#[cfg(all(feature = "python", feature = "screenshot"))]
impl From<screenshot::ScreenshotError> for PyErr {
    fn from(err: screenshot::ScreenshotError) -> PyErr {
        use screenshot::ScreenshotError;
        match err {
            ScreenshotError::IoError(e) => PyIOError::new_err(e.to_string()),
            other => PyRuntimeError::new_err(other.to_string()),
        }
    }
}

/// Convert GraphicsError to PyErr (QA-009)
#[cfg(feature = "python")]
impl From<graphics::GraphicsError> for PyErr {
    fn from(err: graphics::GraphicsError) -> PyErr {
        PyRuntimeError::new_err(err.to_string())
    }
}
