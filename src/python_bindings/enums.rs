//! Python enums for cursor and underline styles
//!
//! This module contains enum definitions for cursor styles (DECSCUSR)
//! and underline styles (SGR 4:x) that are used throughout the terminal API.

use pyo3::prelude::*;

/// Cursor style/shape (DECSCUSR)
#[pyclass(name = "CursorStyle", from_py_object)]
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
#[pyclass(name = "UnderlineStyle", from_py_object)]
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
#[pyclass(name = "MouseEncoding", from_py_object)]
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
/// - Error: Progress with error status (e.g., red)
/// - Indeterminate: Busy/loading indicator (no specific percentage)
/// - Warning: Progress with warning/paused status (e.g., yellow)
#[pyclass(name = "ProgressState", eq, skip_from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyProgressState {
    /// Progress bar is hidden (state 0)
    Hidden = 0,
    /// Normal progress display (state 1)
    Normal = 1,
    /// Error state - operation failed (state 2)
    Error = 2,
    /// Indeterminate/busy indicator (state 3)
    Indeterminate = 3,
    /// Warning/paused state - operation may have issues (state 4)
    Warning = 4,
}

impl<'a, 'py> FromPyObject<'a, 'py> for PyProgressState {
    type Error = PyErr;
    fn extract(ob: pyo3::Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let bound: &Bound<'py, PyAny> = &ob;
        let val: &Bound<'py, Self> = bound.cast()?;
        Ok(*val.borrow())
    }
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

/// Unicode version for character width calculation tables.
///
/// Different Unicode versions have different character width assignments,
/// particularly for newly added emoji and other characters.
#[pyclass(name = "UnicodeVersion", eq, from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyUnicodeVersion {
    /// Unicode 9.0 (June 2016) - Pre-emoji standardization
    Unicode9 = 9,
    /// Unicode 10.0 (June 2017)
    Unicode10 = 10,
    /// Unicode 11.0 (June 2018)
    Unicode11 = 11,
    /// Unicode 12.0 (March 2019)
    Unicode12 = 12,
    /// Unicode 13.0 (March 2020)
    Unicode13 = 13,
    /// Unicode 14.0 (September 2021)
    Unicode14 = 14,
    /// Unicode 15.0 (September 2022)
    Unicode15 = 15,
    /// Unicode 15.1 (September 2023)
    Unicode15_1 = 151,
    /// Unicode 16.0 (September 2024)
    Unicode16 = 16,
    /// Use the latest available Unicode version (default)
    Auto = 0,
}

#[pymethods]
impl PyUnicodeVersion {
    /// Get a human-readable version string
    fn version_string(&self) -> &'static str {
        match self {
            PyUnicodeVersion::Unicode9 => "9.0",
            PyUnicodeVersion::Unicode10 => "10.0",
            PyUnicodeVersion::Unicode11 => "11.0",
            PyUnicodeVersion::Unicode12 => "12.0",
            PyUnicodeVersion::Unicode13 => "13.0",
            PyUnicodeVersion::Unicode14 => "14.0",
            PyUnicodeVersion::Unicode15 => "15.0",
            PyUnicodeVersion::Unicode15_1 => "15.1",
            PyUnicodeVersion::Unicode16 => "16.0",
            PyUnicodeVersion::Auto => "auto",
        }
    }

    /// Check if this is the Auto setting
    fn is_auto(&self) -> bool {
        matches!(self, PyUnicodeVersion::Auto)
    }

    fn __repr__(&self) -> String {
        format!(
            "UnicodeVersion.{}",
            self.version_string().replace('.', "_").to_uppercase()
        )
    }
}

impl From<crate::unicode_width_config::UnicodeVersion> for PyUnicodeVersion {
    fn from(version: crate::unicode_width_config::UnicodeVersion) -> Self {
        match version {
            crate::unicode_width_config::UnicodeVersion::Unicode9 => PyUnicodeVersion::Unicode9,
            crate::unicode_width_config::UnicodeVersion::Unicode10 => PyUnicodeVersion::Unicode10,
            crate::unicode_width_config::UnicodeVersion::Unicode11 => PyUnicodeVersion::Unicode11,
            crate::unicode_width_config::UnicodeVersion::Unicode12 => PyUnicodeVersion::Unicode12,
            crate::unicode_width_config::UnicodeVersion::Unicode13 => PyUnicodeVersion::Unicode13,
            crate::unicode_width_config::UnicodeVersion::Unicode14 => PyUnicodeVersion::Unicode14,
            crate::unicode_width_config::UnicodeVersion::Unicode15 => PyUnicodeVersion::Unicode15,
            crate::unicode_width_config::UnicodeVersion::Unicode15_1 => {
                PyUnicodeVersion::Unicode15_1
            }
            crate::unicode_width_config::UnicodeVersion::Unicode16 => PyUnicodeVersion::Unicode16,
            crate::unicode_width_config::UnicodeVersion::Auto => PyUnicodeVersion::Auto,
        }
    }
}

impl From<PyUnicodeVersion> for crate::unicode_width_config::UnicodeVersion {
    fn from(version: PyUnicodeVersion) -> Self {
        match version {
            PyUnicodeVersion::Unicode9 => crate::unicode_width_config::UnicodeVersion::Unicode9,
            PyUnicodeVersion::Unicode10 => crate::unicode_width_config::UnicodeVersion::Unicode10,
            PyUnicodeVersion::Unicode11 => crate::unicode_width_config::UnicodeVersion::Unicode11,
            PyUnicodeVersion::Unicode12 => crate::unicode_width_config::UnicodeVersion::Unicode12,
            PyUnicodeVersion::Unicode13 => crate::unicode_width_config::UnicodeVersion::Unicode13,
            PyUnicodeVersion::Unicode14 => crate::unicode_width_config::UnicodeVersion::Unicode14,
            PyUnicodeVersion::Unicode15 => crate::unicode_width_config::UnicodeVersion::Unicode15,
            PyUnicodeVersion::Unicode15_1 => {
                crate::unicode_width_config::UnicodeVersion::Unicode15_1
            }
            PyUnicodeVersion::Unicode16 => crate::unicode_width_config::UnicodeVersion::Unicode16,
            PyUnicodeVersion::Auto => crate::unicode_width_config::UnicodeVersion::Auto,
        }
    }
}

/// Treatment of East Asian Ambiguous width characters.
///
/// Ambiguous characters include Greek/Cyrillic letters, some symbols, and
/// other characters that may display as either 1 or 2 cells depending on context.
#[pyclass(name = "AmbiguousWidth", eq, from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyAmbiguousWidth {
    /// Narrow (1 cell) - Western/default terminal behavior
    Narrow = 1,
    /// Wide (2 cells) - CJK terminal behavior
    Wide = 2,
}

#[pymethods]
impl PyAmbiguousWidth {
    /// Get the width value (1 or 2)
    fn width(&self) -> usize {
        match self {
            PyAmbiguousWidth::Narrow => 1,
            PyAmbiguousWidth::Wide => 2,
        }
    }

    /// Check if this is the narrow setting
    fn is_narrow(&self) -> bool {
        matches!(self, PyAmbiguousWidth::Narrow)
    }

    /// Check if this is the wide setting
    fn is_wide(&self) -> bool {
        matches!(self, PyAmbiguousWidth::Wide)
    }

    fn __repr__(&self) -> String {
        match self {
            PyAmbiguousWidth::Narrow => "AmbiguousWidth.NARROW".to_string(),
            PyAmbiguousWidth::Wide => "AmbiguousWidth.WIDE".to_string(),
        }
    }
}

impl From<crate::unicode_width_config::AmbiguousWidth> for PyAmbiguousWidth {
    fn from(width: crate::unicode_width_config::AmbiguousWidth) -> Self {
        match width {
            crate::unicode_width_config::AmbiguousWidth::Narrow => PyAmbiguousWidth::Narrow,
            crate::unicode_width_config::AmbiguousWidth::Wide => PyAmbiguousWidth::Wide,
        }
    }
}

impl From<PyAmbiguousWidth> for crate::unicode_width_config::AmbiguousWidth {
    fn from(width: PyAmbiguousWidth) -> Self {
        match width {
            PyAmbiguousWidth::Narrow => crate::unicode_width_config::AmbiguousWidth::Narrow,
            PyAmbiguousWidth::Wide => crate::unicode_width_config::AmbiguousWidth::Wide,
        }
    }
}

/// Configuration for Unicode width calculations.
///
/// This class combines Unicode version and ambiguous width settings
/// to control how character widths are calculated in the terminal.
#[pyclass(name = "WidthConfig", from_py_object)]
#[derive(Debug, Clone)]
pub struct PyWidthConfig {
    /// Unicode version for width tables
    #[pyo3(get, set)]
    pub unicode_version: PyUnicodeVersion,
    /// Treatment of East Asian Ambiguous width characters
    #[pyo3(get, set)]
    pub ambiguous_width: PyAmbiguousWidth,
}

#[pymethods]
impl PyWidthConfig {
    /// Create a new WidthConfig with specified settings
    ///
    /// Args:
    ///     unicode_version: Unicode version for width tables (default: Auto)
    ///     ambiguous_width: Treatment of ambiguous characters (default: Narrow)
    #[new]
    #[pyo3(signature = (unicode_version=None, ambiguous_width=None))]
    fn new(
        unicode_version: Option<PyUnicodeVersion>,
        ambiguous_width: Option<PyAmbiguousWidth>,
    ) -> Self {
        Self {
            unicode_version: unicode_version.unwrap_or(PyUnicodeVersion::Auto),
            ambiguous_width: ambiguous_width.unwrap_or(PyAmbiguousWidth::Narrow),
        }
    }

    /// Create a WidthConfig optimized for CJK environments
    ///
    /// Returns:
    ///     WidthConfig with Auto Unicode version and Wide ambiguous width
    #[staticmethod]
    fn cjk() -> Self {
        Self {
            unicode_version: PyUnicodeVersion::Auto,
            ambiguous_width: PyAmbiguousWidth::Wide,
        }
    }

    /// Create a WidthConfig optimized for Western environments
    ///
    /// Returns:
    ///     WidthConfig with Auto Unicode version and Narrow ambiguous width
    #[staticmethod]
    fn western() -> Self {
        Self {
            unicode_version: PyUnicodeVersion::Auto,
            ambiguous_width: PyAmbiguousWidth::Narrow,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "WidthConfig(unicode_version={}, ambiguous_width={})",
            self.unicode_version.__repr__(),
            self.ambiguous_width.__repr__()
        )
    }
}

impl From<crate::unicode_width_config::WidthConfig> for PyWidthConfig {
    fn from(config: crate::unicode_width_config::WidthConfig) -> Self {
        Self {
            unicode_version: config.unicode_version.into(),
            ambiguous_width: config.ambiguous_width.into(),
        }
    }
}

impl From<PyWidthConfig> for crate::unicode_width_config::WidthConfig {
    fn from(config: PyWidthConfig) -> Self {
        Self {
            unicode_version: config.unicode_version.into(),
            ambiguous_width: config.ambiguous_width.into(),
        }
    }
}

impl From<&PyWidthConfig> for crate::unicode_width_config::WidthConfig {
    fn from(config: &PyWidthConfig) -> Self {
        Self {
            unicode_version: config.unicode_version.into(),
            ambiguous_width: config.ambiguous_width.into(),
        }
    }
}

/// Unicode normalization form for terminal text.
///
/// Controls how Unicode text is normalized before being stored in terminal cells.
/// Normalization ensures consistent representation for search and comparison.
#[pyclass(name = "NormalizationForm", eq, from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyNormalizationForm {
    /// No normalization - store text as received
    Disabled = 0,
    /// Canonical Decomposition, followed by Canonical Composition (default)
    NFC = 1,
    /// Canonical Decomposition
    NFD = 2,
    /// Compatibility Decomposition, followed by Canonical Composition
    NFKC = 3,
    /// Compatibility Decomposition
    NFKD = 4,
}

#[pymethods]
impl PyNormalizationForm {
    /// Get a human-readable name for the normalization form
    fn name(&self) -> &'static str {
        match self {
            PyNormalizationForm::Disabled => "none",
            PyNormalizationForm::NFC => "NFC",
            PyNormalizationForm::NFD => "NFD",
            PyNormalizationForm::NFKC => "NFKC",
            PyNormalizationForm::NFKD => "NFKD",
        }
    }

    /// Check if normalization is disabled
    fn is_none(&self) -> bool {
        matches!(self, PyNormalizationForm::Disabled)
    }

    fn __repr__(&self) -> String {
        match self {
            PyNormalizationForm::Disabled => "NormalizationForm.Disabled".to_string(),
            _ => format!("NormalizationForm.{}", self.name()),
        }
    }
}

impl From<crate::unicode_normalization_config::NormalizationForm> for PyNormalizationForm {
    fn from(form: crate::unicode_normalization_config::NormalizationForm) -> Self {
        match form {
            crate::unicode_normalization_config::NormalizationForm::None => {
                PyNormalizationForm::Disabled
            }
            crate::unicode_normalization_config::NormalizationForm::NFC => PyNormalizationForm::NFC,
            crate::unicode_normalization_config::NormalizationForm::NFD => PyNormalizationForm::NFD,
            crate::unicode_normalization_config::NormalizationForm::NFKC => {
                PyNormalizationForm::NFKC
            }
            crate::unicode_normalization_config::NormalizationForm::NFKD => {
                PyNormalizationForm::NFKD
            }
        }
    }
}

impl From<PyNormalizationForm> for crate::unicode_normalization_config::NormalizationForm {
    fn from(form: PyNormalizationForm) -> Self {
        match form {
            PyNormalizationForm::Disabled => {
                crate::unicode_normalization_config::NormalizationForm::None
            }
            PyNormalizationForm::NFC => crate::unicode_normalization_config::NormalizationForm::NFC,
            PyNormalizationForm::NFD => crate::unicode_normalization_config::NormalizationForm::NFD,
            PyNormalizationForm::NFKC => {
                crate::unicode_normalization_config::NormalizationForm::NFKC
            }
            PyNormalizationForm::NFKD => {
                crate::unicode_normalization_config::NormalizationForm::NFKD
            }
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
