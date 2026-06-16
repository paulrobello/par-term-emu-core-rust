//! Search & content-detection API methods for `PyTerminal` (ARC-002: split out
//! of the monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no
//! Python API or behavior change; these methods remain on the same `Terminal`
//! Python class.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Search Methods ===

    /// Search for text in the visible screen
    ///
    /// Args:
    ///     query: Text to search for
    ///     case_sensitive: Whether the search should be case-sensitive
    ///
    /// Returns:
    ///     List of SearchMatch objects with position and matched text
    #[pyo3(signature = (query, case_sensitive=false))]
    fn search(
        &mut self,
        query: &str,
        case_sensitive: bool,
    ) -> PyResult<Vec<crate::python_bindings::types::PySearchMatch>> {
        use crate::terminal::search::RegexSearchOptions;
        let options = RegexSearchOptions {
            case_insensitive: !case_sensitive,
            ..Default::default()
        };
        let matches = self
            .inner
            .search(query, options)
            .map_err(PyRuntimeError::new_err)?;
        Ok(matches
            .iter()
            .map(|m| crate::python_bindings::types::PySearchMatch {
                row: m.row as isize,
                col: m.col,
                length: m.length,
                text: m.text.clone(),
            })
            .collect())
    }

    /// Search for text in the scrollback buffer
    ///
    /// Args:
    ///     query: Text to search for
    ///     case_sensitive: Whether the search should be case-sensitive
    ///     max_lines: Maximum number of scrollback lines to search (None = all)
    ///
    /// Returns:
    ///     List of SearchMatch objects with negative row indices for scrollback
    #[pyo3(signature = (query, case_sensitive=false, max_lines=None))]
    fn search_scrollback(
        &self,
        query: &str,
        case_sensitive: bool,
        max_lines: Option<usize>,
    ) -> PyResult<Vec<crate::python_bindings::types::PySearchMatch>> {
        let matches = self
            .inner
            .search_scrollback(query, case_sensitive, max_lines);
        Ok(matches
            .iter()
            .map(|m| crate::python_bindings::types::PySearchMatch {
                row: m.row,
                col: m.col,
                length: m.length,
                text: m.text.clone(),
            })
            .collect())
    }

    // === Content Detection Methods ===

    /// Detect URLs in the visible screen
    ///
    /// Returns:
    ///     List of DetectedItem objects for URLs
    fn detect_urls(&self) -> PyResult<Vec<crate::python_bindings::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_urls();
        Ok(items
            .iter()
            .map(|item| match item {
                DetectedItem::Url(text, row, col) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "url".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: None,
                    }
                }
                _ => unreachable!(),
            })
            .collect())
    }

    /// Detect file paths in the visible screen
    ///
    /// Returns:
    ///     List of DetectedItem objects for file paths
    fn detect_file_paths(&self) -> PyResult<Vec<crate::python_bindings::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_file_paths();
        Ok(items
            .iter()
            .map(|item| match item {
                DetectedItem::FilePath(text, row, col, line_num) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "filepath".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: *line_num,
                    }
                }
                _ => unreachable!(),
            })
            .collect())
    }

    /// Detect semantic items (URLs, file paths, git hashes, IPs, emails)
    ///
    /// Returns:
    ///     List of all detected semantic items
    fn detect_semantic_items(
        &self,
    ) -> PyResult<Vec<crate::python_bindings::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_semantic_items();
        Ok(items
            .iter()
            .map(|item| match item {
                DetectedItem::Url(text, row, col) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "url".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: None,
                    }
                }
                DetectedItem::FilePath(text, row, col, line_num) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "filepath".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: *line_num,
                    }
                }
                DetectedItem::GitHash(text, row, col) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "git_hash".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: None,
                    }
                }
                DetectedItem::IpAddress(text, row, col) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "ip".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: None,
                    }
                }
                DetectedItem::Email(text, row, col) => {
                    crate::python_bindings::types::PyDetectedItem {
                        item_type: "email".to_string(),
                        text: text.clone(),
                        row: *row,
                        col: *col,
                        line_number: None,
                    }
                }
            })
            .collect())
    }

    // === Feature 15: Regex Search ===

    /// Perform regex search on terminal content
    #[pyo3(signature = (pattern, case_insensitive=false, multiline=true, include_scrollback=true, max_matches=0, reverse=false))]
    fn regex_search(
        &mut self,
        pattern: &str,
        case_insensitive: bool,
        multiline: bool,
        include_scrollback: bool,
        max_matches: usize,
        reverse: bool,
    ) -> PyResult<Vec<crate::python_bindings::types::PyRegexMatch>> {
        use crate::terminal::RegexSearchOptions;

        let options = RegexSearchOptions {
            case_insensitive,
            multiline,
            include_scrollback,
            max_matches,
            reverse,
        };

        let matches = self
            .inner
            .regex_search(pattern, options)
            .map_err(PyValueError::new_err)?;

        Ok(matches
            .iter()
            .map(crate::python_bindings::types::PyRegexMatch::from)
            .collect())
    }

    /// Get cached regex matches
    fn get_regex_matches(&self) -> PyResult<Vec<crate::python_bindings::types::PyRegexMatch>> {
        Ok(self
            .inner
            .get_regex_matches()
            .iter()
            .map(crate::python_bindings::types::PyRegexMatch::from)
            .collect())
    }

    /// Get current regex search pattern
    fn get_current_regex_pattern(&self) -> PyResult<Option<String>> {
        Ok(self.inner.get_current_regex_pattern())
    }

    /// Clear regex search cache
    fn clear_regex_matches(&mut self) -> PyResult<()> {
        self.inner.clear_regex_matches();
        Ok(())
    }

    /// Find next regex match from a position
    fn next_regex_match(
        &self,
        from_row: usize,
        from_col: usize,
    ) -> PyResult<Option<crate::python_bindings::types::PyRegexMatch>> {
        Ok(self
            .inner
            .next_regex_match(from_row, from_col)
            .map(|m| crate::python_bindings::types::PyRegexMatch::from(&m)))
    }

    /// Find previous regex match from a position
    fn prev_regex_match(
        &self,
        from_row: usize,
        from_col: usize,
    ) -> PyResult<Option<crate::python_bindings::types::PyRegexMatch>> {
        Ok(self
            .inner
            .prev_regex_match(from_row, from_col)
            .map(|m| crate::python_bindings::types::PyRegexMatch::from(&m)))
    }
}
