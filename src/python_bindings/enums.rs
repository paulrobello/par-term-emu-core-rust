//! Python enums for cursor and underline styles
//!
//! This module contains enum definitions for cursor styles (DECSCUSR)
//! and underline styles (SGR 4:x) that are used throughout the terminal API.

use pyo3::prelude::*;

/// Cursor style/shape (DECSCUSR)
#[pyclass(name = "CursorStyle")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyCursorStyle {
    /// Blinking block (default)
    BlinkingBlock = 1,
    /// Steady block
    SteadyBlock = 2,
    /// Blinking underline
    BlinkingUnderline = 3,
    /// Steady underline
    SteadyUnderline = 4,
    /// Blinking bar (I-beam)
    BlinkingBar = 5,
    /// Steady bar (I-beam)
    SteadyBar = 6,
}

impl From<crate::cursor::CursorStyle> for PyCursorStyle {
    fn from(style: crate::cursor::CursorStyle) -> Self {
        match style {
            crate::cursor::CursorStyle::BlinkingBlock => PyCursorStyle::BlinkingBlock,
            crate::cursor::CursorStyle::SteadyBlock => PyCursorStyle::SteadyBlock,
            crate::cursor::CursorStyle::BlinkingUnderline => PyCursorStyle::BlinkingUnderline,
            crate::cursor::CursorStyle::SteadyUnderline => PyCursorStyle::SteadyUnderline,
            crate::cursor::CursorStyle::BlinkingBar => PyCursorStyle::BlinkingBar,
            crate::cursor::CursorStyle::SteadyBar => PyCursorStyle::SteadyBar,
        }
    }
}

/// Underline style for text decoration (SGR 4:x)
#[pyclass(name = "UnderlineStyle")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyUnderlineStyle {
    /// No underline
    None = 0,
    /// Straight/single underline (default)
    Straight = 1,
    /// Double underline
    Double = 2,
    /// Curly underline (for spell check, errors)
    Curly = 3,
    /// Dotted underline
    Dotted = 4,
    /// Dashed underline
    Dashed = 5,
}

impl From<crate::cell::UnderlineStyle> for PyUnderlineStyle {
    fn from(style: crate::cell::UnderlineStyle) -> Self {
        match style {
            crate::cell::UnderlineStyle::None => PyUnderlineStyle::None,
            crate::cell::UnderlineStyle::Straight => PyUnderlineStyle::Straight,
            crate::cell::UnderlineStyle::Double => PyUnderlineStyle::Double,
            crate::cell::UnderlineStyle::Curly => PyUnderlineStyle::Curly,
            crate::cell::UnderlineStyle::Dotted => PyUnderlineStyle::Dotted,
            crate::cell::UnderlineStyle::Dashed => PyUnderlineStyle::Dashed,
        }
    }
}

/// Mouse encoding format for mouse event reporting
#[pyclass(name = "MouseEncoding")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyMouseEncoding {
    /// Default X11 encoding (values 32-255)
    Default = 0,
    /// UTF-8 encoding (supports larger coordinates)
    Utf8 = 1,
    /// SGR encoding (1006) - recommended for modern terminals
    Sgr = 2,
    /// URXVT encoding (1015)
    Urxvt = 3,
}

impl From<crate::mouse::MouseEncoding> for PyMouseEncoding {
    fn from(encoding: crate::mouse::MouseEncoding) -> Self {
        match encoding {
            crate::mouse::MouseEncoding::Default => PyMouseEncoding::Default,
            crate::mouse::MouseEncoding::Utf8 => PyMouseEncoding::Utf8,
            crate::mouse::MouseEncoding::Sgr => PyMouseEncoding::Sgr,
            crate::mouse::MouseEncoding::Urxvt => PyMouseEncoding::Urxvt,
        }
    }
}

impl From<PyMouseEncoding> for crate::mouse::MouseEncoding {
    fn from(encoding: PyMouseEncoding) -> Self {
        match encoding {
            PyMouseEncoding::Default => crate::mouse::MouseEncoding::Default,
            PyMouseEncoding::Utf8 => crate::mouse::MouseEncoding::Utf8,
            PyMouseEncoding::Sgr => crate::mouse::MouseEncoding::Sgr,
            PyMouseEncoding::Urxvt => crate::mouse::MouseEncoding::Urxvt,
        }
    }
}

/// Progress bar state from OSC 9;4 sequences (ConEmu/Windows Terminal style)
///
/// This enum represents the different visual states of a progress bar:
/// - Hidden: Progress bar is not displayed
/// - Normal: Standard progress indicator (0-100%)
/// - Indeterminate: Busy/loading indicator (no specific percentage)
/// - Warning: Progress with warning status (e.g., yellow)
/// - Error: Progress with error status (e.g., red)
#[pyclass(name = "ProgressState", eq)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyProgressState {
    /// Progress bar is hidden (state 0)
    Hidden = 0,
    /// Normal progress display (state 1)
    Normal = 1,
    /// Indeterminate/busy indicator (state 2)
    Indeterminate = 2,
    /// Warning state - operation may have issues (state 3)
    Warning = 3,
    /// Error state - operation failed (state 4)
    Error = 4,
}

impl PyProgressState {
    /// Check if the state represents an active (visible) progress bar
    pub fn is_active(&self) -> bool {
        !matches!(self, PyProgressState::Hidden)
    }

    /// Check if the state requires a progress percentage
    pub fn requires_progress(&self) -> bool {
        matches!(
            self,
            PyProgressState::Normal | PyProgressState::Warning | PyProgressState::Error
        )
    }
}

#[pymethods]
impl PyProgressState {
    /// Get a human-readable description of the state
    fn description(&self) -> &'static str {
        match self {
            PyProgressState::Hidden => "hidden",
            PyProgressState::Normal => "normal",
            PyProgressState::Indeterminate => "indeterminate",
            PyProgressState::Warning => "warning",
            PyProgressState::Error => "error",
        }
    }

    fn __repr__(&self) -> String {
        format!("ProgressState.{}", self.description().to_uppercase())
    }
}

impl From<crate::terminal::ProgressState> for PyProgressState {
    fn from(state: crate::terminal::ProgressState) -> Self {
        match state {
            crate::terminal::ProgressState::Hidden => PyProgressState::Hidden,
            crate::terminal::ProgressState::Normal => PyProgressState::Normal,
            crate::terminal::ProgressState::Indeterminate => PyProgressState::Indeterminate,
            crate::terminal::ProgressState::Warning => PyProgressState::Warning,
            crate::terminal::ProgressState::Error => PyProgressState::Error,
        }
    }
}

impl From<PyProgressState> for crate::terminal::ProgressState {
    fn from(state: PyProgressState) -> Self {
        match state {
            PyProgressState::Hidden => crate::terminal::ProgressState::Hidden,
            PyProgressState::Normal => crate::terminal::ProgressState::Normal,
            PyProgressState::Indeterminate => crate::terminal::ProgressState::Indeterminate,
            PyProgressState::Warning => crate::terminal::ProgressState::Warning,
            PyProgressState::Error => crate::terminal::ProgressState::Error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_style_from_rust_blinking_block() {
        let rust_style = crate::cursor::CursorStyle::BlinkingBlock;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::BlinkingBlock);
        assert_eq!(py_style as u8, 1);
    }

    #[test]
    fn test_cursor_style_from_rust_steady_block() {
        let rust_style = crate::cursor::CursorStyle::SteadyBlock;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::SteadyBlock);
        assert_eq!(py_style as u8, 2);
    }

    #[test]
    fn test_cursor_style_from_rust_blinking_underline() {
        let rust_style = crate::cursor::CursorStyle::BlinkingUnderline;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::BlinkingUnderline);
        assert_eq!(py_style as u8, 3);
    }

    #[test]
    fn test_cursor_style_from_rust_steady_underline() {
        let rust_style = crate::cursor::CursorStyle::SteadyUnderline;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::SteadyUnderline);
        assert_eq!(py_style as u8, 4);
    }

    #[test]
    fn test_cursor_style_from_rust_blinking_bar() {
        let rust_style = crate::cursor::CursorStyle::BlinkingBar;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::BlinkingBar);
        assert_eq!(py_style as u8, 5);
    }

    #[test]
    fn test_cursor_style_from_rust_steady_bar() {
        let rust_style = crate::cursor::CursorStyle::SteadyBar;
        let py_style: PyCursorStyle = rust_style.into();
        assert_eq!(py_style, PyCursorStyle::SteadyBar);
        assert_eq!(py_style as u8, 6);
    }

    #[test]
    fn test_cursor_style_all_variants() {
        // Ensure all Rust variants are covered
        let variants = vec![
            crate::cursor::CursorStyle::BlinkingBlock,
            crate::cursor::CursorStyle::SteadyBlock,
            crate::cursor::CursorStyle::BlinkingUnderline,
            crate::cursor::CursorStyle::SteadyUnderline,
            crate::cursor::CursorStyle::BlinkingBar,
            crate::cursor::CursorStyle::SteadyBar,
        ];

        for variant in variants {
            let _py_style: PyCursorStyle = variant.into();
            // Successfully converts all variants
        }
    }

    #[test]
    fn test_cursor_style_values_match_decscusr() {
        // DECSCUSR spec values
        assert_eq!(PyCursorStyle::BlinkingBlock as u8, 1);
        assert_eq!(PyCursorStyle::SteadyBlock as u8, 2);
        assert_eq!(PyCursorStyle::BlinkingUnderline as u8, 3);
        assert_eq!(PyCursorStyle::SteadyUnderline as u8, 4);
        assert_eq!(PyCursorStyle::BlinkingBar as u8, 5);
        assert_eq!(PyCursorStyle::SteadyBar as u8, 6);
    }

    #[test]
    fn test_underline_style_from_rust_none() {
        let rust_style = crate::cell::UnderlineStyle::None;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::None);
        assert_eq!(py_style as u8, 0);
    }

    #[test]
    fn test_underline_style_from_rust_straight() {
        let rust_style = crate::cell::UnderlineStyle::Straight;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::Straight);
        assert_eq!(py_style as u8, 1);
    }

    #[test]
    fn test_underline_style_from_rust_double() {
        let rust_style = crate::cell::UnderlineStyle::Double;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::Double);
        assert_eq!(py_style as u8, 2);
    }

    #[test]
    fn test_underline_style_from_rust_curly() {
        let rust_style = crate::cell::UnderlineStyle::Curly;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::Curly);
        assert_eq!(py_style as u8, 3);
    }

    #[test]
    fn test_underline_style_from_rust_dotted() {
        let rust_style = crate::cell::UnderlineStyle::Dotted;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::Dotted);
        assert_eq!(py_style as u8, 4);
    }

    #[test]
    fn test_underline_style_from_rust_dashed() {
        let rust_style = crate::cell::UnderlineStyle::Dashed;
        let py_style: PyUnderlineStyle = rust_style.into();
        assert_eq!(py_style, PyUnderlineStyle::Dashed);
        assert_eq!(py_style as u8, 5);
    }

    #[test]
    fn test_underline_style_all_variants() {
        // Ensure all Rust variants are covered
        let variants = vec![
            crate::cell::UnderlineStyle::None,
            crate::cell::UnderlineStyle::Straight,
            crate::cell::UnderlineStyle::Double,
            crate::cell::UnderlineStyle::Curly,
            crate::cell::UnderlineStyle::Dotted,
            crate::cell::UnderlineStyle::Dashed,
        ];

        for variant in variants {
            let _py_style: PyUnderlineStyle = variant.into();
            // Successfully converts all variants
        }
    }

    #[test]
    fn test_underline_style_values_match_sgr() {
        // SGR 4:x spec values
        assert_eq!(PyUnderlineStyle::None as u8, 0);
        assert_eq!(PyUnderlineStyle::Straight as u8, 1);
        assert_eq!(PyUnderlineStyle::Double as u8, 2);
        assert_eq!(PyUnderlineStyle::Curly as u8, 3);
        assert_eq!(PyUnderlineStyle::Dotted as u8, 4);
        assert_eq!(PyUnderlineStyle::Dashed as u8, 5);
    }

    #[test]
    fn test_py_cursor_style_clone() {
        let style = PyCursorStyle::BlinkingBlock;
        let cloned = style;
        assert_eq!(style, cloned);
    }

    #[test]
    fn test_py_underline_style_clone() {
        let style = PyUnderlineStyle::Curly;
        let cloned = style;
        assert_eq!(style, cloned);
    }

    #[test]
    fn test_py_cursor_style_debug() {
        let style = PyCursorStyle::SteadyBar;
        let debug_str = format!("{:?}", style);
        assert!(debug_str.contains("SteadyBar"));
    }

    #[test]
    fn test_py_underline_style_debug() {
        let style = PyUnderlineStyle::Double;
        let debug_str = format!("{:?}", style);
        assert!(debug_str.contains("Double"));
    }

    #[test]
    fn test_enum_equality() {
        assert_eq!(PyCursorStyle::BlinkingBlock, PyCursorStyle::BlinkingBlock);
        assert_ne!(PyCursorStyle::BlinkingBlock, PyCursorStyle::SteadyBlock);

        assert_eq!(PyUnderlineStyle::Curly, PyUnderlineStyle::Curly);
        assert_ne!(PyUnderlineStyle::Curly, PyUnderlineStyle::Dotted);
    }
}
