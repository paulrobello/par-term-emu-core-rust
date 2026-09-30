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
    ///     query: Regular expression to search for (escape metacharacters
    ///         such as ``.`` or ``(`` to match them literally)
    ///     case_sensitive: Whether the search should be case-sensitive
    ///
    /// Returns:
    ///     List of SearchMatch objects with position and matched text
    ///
    /// Raises:
    ///     RuntimeError: If the query is not a valid regular expression
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("first\r\nsay hello\r\n")
    ///     term.search("hello")   # [SearchMatch(row=1, col=4, length=5, text="hello")]
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 2)
    ///     term.process_str("error: disk full\r\nok\r\n")   # first line scrolls off
    ///     term.search_scrollback("disk")   # [SearchMatch(row=-1, col=7, length=4, text="disk")]
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("docs at https://example.com\r\n")
    ///     [(i.item_type, i.text) for i in term.detect_urls()]   # [('url', 'https://example.com')]
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("error in /usr/src/app/main.py:42\r\n")
    ///     [(i.text, i.line_number) for i in term.detect_file_paths()]
    ///     # [('/usr/src/app/main.py', 42)]
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("mail admin@example.com from 192.168.1.10\r\n")
    ///     [(i.item_type, i.text) for i in term.detect_semantic_items()]
    ///     # [('ip', '192.168.1.10'), ('email', 'admin@example.com')]
    ///     ```
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
    ///
    /// Args:
    ///     pattern: Regex pattern to search for
    ///     case_insensitive: Match without regard to case (default False)
    ///     multiline: Let ^ and $ match line boundaries (default True)
    ///     include_scrollback: Search scrollback as well as the screen (default True)
    ///     max_matches: Stop after this many matches, 0 for unlimited
    ///     reverse: Search from the end backwards (default False)
    ///
    /// Returns:
    ///     list[RegexMatch]: Matches found, in search order
    ///
    /// Raises:
    ///     ValueError: If the pattern is not a valid regex
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("id=42 id=7\r\n")
    ///     term.regex_search(r"id=(\d+)")
    ///     # [RegexMatch(row=0, col=0, text="id=42"), RegexMatch(row=0, col=6, text="id=7")]
    ///     ```
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
    ///
    /// Returns:
    ///     list[RegexMatch]: Matches from the most recent regex search
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("id=42 id=7\r\n")
    ///     term.regex_search(r"id=(\d+)")
    ///     [m.text for m in term.get_regex_matches()]   # ['id=42', 'id=7']
    ///     ```
    fn get_regex_matches(&self) -> PyResult<Vec<crate::python_bindings::types::PyRegexMatch>> {
        Ok(self
            .inner
            .get_regex_matches()
            .iter()
            .map(crate::python_bindings::types::PyRegexMatch::from)
            .collect())
    }

    /// Get current regex search pattern
    ///
    /// Returns:
    ///     str | None: The active regex pattern, or None if no search ran
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_current_regex_pattern()   # None
    ///     term.regex_search("id=[0-9]+")
    ///     term.get_current_regex_pattern()   # 'id=[0-9]+'
    ///     ```
    fn get_current_regex_pattern(&self) -> PyResult<Option<String>> {
        Ok(self.inner.get_current_regex_pattern())
    }

    /// Clear regex search cache
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("id=42 id=7\r\n")
    ///     term.regex_search(r"id=(\d+)")
    ///     term.clear_regex_matches()
    ///     term.get_regex_matches()           # []
    ///     term.get_current_regex_pattern()   # None
    ///     ```
    fn clear_regex_matches(&mut self) -> PyResult<()> {
        self.inner.clear_regex_matches();
        Ok(())
    }

    /// Find next regex match from a position
    ///
    /// Args:
    ///     from_row: Row to search from (0-indexed)
    ///     from_col: Column to search from (0-indexed)
    ///
    /// Returns:
    ///     RegexMatch | None: Next cached match strictly after the position, if any
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("id=42 id=7\r\n")
    ///     term.regex_search(r"id=(\d+)")
    ///     term.next_regex_match(0, 0)   # RegexMatch(row=0, col=6, text="id=7")
    ///     term.next_regex_match(0, 6)   # None
    ///     ```
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
    ///
    /// Args:
    ///     from_row: Row to search from (0-indexed)
    ///     from_col: Column to search from (0-indexed)
    ///
    /// Returns:
    ///     RegexMatch | None: Previous cached match strictly before the position, if any
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.process_str("id=42 id=7\r\n")
    ///     term.regex_search(r"id=(\d+)")
    ///     term.prev_regex_match(0, 6)   # RegexMatch(row=0, col=0, text="id=42")
    ///     term.prev_regex_match(0, 0)   # None
    ///     ```
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
