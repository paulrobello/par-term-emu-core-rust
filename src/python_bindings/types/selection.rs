//! Search, selection, and detected-item types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

impl From<crate::terminal::search::RegexMatch> for PySearchMatch {
    fn from(m: crate::terminal::search::RegexMatch) -> Self {
        PySearchMatch {
            row: m.row as isize,
            col: m.col,
            length: m.length,
            text: m.text,
        }
    }
}

/// Search match result
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "SearchMatch", from_py_object)]
#[derive(Clone)]
pub struct PySearchMatch {
    /// Row index (negative for scrollback, 0+ for visible screen)
    pub row: isize,
    /// Column index
    pub col: usize,
    /// Length of the match
    pub length: usize,
    /// Matched text
    pub text: String,
}

#[pymethods]
impl PySearchMatch {
    fn __repr__(&self) -> String {
        format!(
            "SearchMatch(row={}, col={}, length={}, text={:?})",
            self.row, self.col, self.length, self.text
        )
    }
}

/// Detected semantic item
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "DetectedItem", from_py_object)]
#[derive(Clone)]
pub struct PyDetectedItem {
    /// Item type: "url", "filepath", "git_hash", "ip", or "email"
    pub item_type: String,
    /// The detected text
    pub text: String,
    /// Row index
    pub row: usize,
    /// Column index
    pub col: usize,
    /// Optional line number (for file paths like "file.txt:123")
    pub line_number: Option<usize>,
}

#[pymethods]
impl PyDetectedItem {
    fn __repr__(&self) -> String {
        format!(
            "DetectedItem(type={}, text={:?}, row={}, col={})",
            self.item_type, self.text, self.row, self.col
        )
    }
}

/// Selection mode
#[pyclass(name = "SelectionMode", from_py_object)]
#[derive(Clone)]
pub enum PySelectionMode {
    Character,
    Line,
    Block,
}

/// Selection state
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "Selection", from_py_object)]
#[derive(Clone)]
pub struct PySelection {
    /// Start position (col, row)
    pub start: (usize, usize),
    /// End position (col, row)
    pub end: (usize, usize),
    /// Selection mode
    pub mode: String,
}

#[pymethods]
impl PySelection {
    fn __repr__(&self) -> String {
        format!(
            "Selection(start={:?}, end={:?}, mode={})",
            self.start, self.end, self.mode
        )
    }
}

/// Regex match
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "RegexMatch", from_py_object)]
#[derive(Clone)]
pub struct PyRegexMatch {
    pub row: usize,
    pub col: usize,
    pub end_row: usize,
    pub end_col: usize,
    pub text: String,
    pub captures: Vec<String>,
}

#[pymethods]
impl PyRegexMatch {
    fn __repr__(&self) -> String {
        format!(
            "RegexMatch(row={}, col={}, text={:?})",
            self.row, self.col, self.text
        )
    }
}

impl From<&crate::terminal::RegexMatch> for PyRegexMatch {
    fn from(m: &crate::terminal::RegexMatch) -> Self {
        PyRegexMatch {
            row: m.row,
            col: m.col,
            end_row: m.end_row,
            end_col: m.end_col,
            text: m.text.clone(),
            captures: m.captures.clone(),
        }
    }
}
