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
