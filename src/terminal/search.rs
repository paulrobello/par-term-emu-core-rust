//! Terminal search and pattern matching
//!
//! Provides types for search matches, hyperlink information, and regex search.

/// Hyperlink information with all its locations
#[derive(Debug, Clone)]
pub struct HyperlinkInfo {
    /// The URL of the hyperlink
    pub url: String,
    /// All (col, row) positions where this link appears
    pub positions: Vec<(usize, usize)>,
    /// Optional hyperlink ID from OSC 8
    pub id: Option<String>,
}

/// Search match result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// Row index (negative values are scrollback, 0+ are visible screen)
    pub row: isize,
    /// Column where match starts (0-indexed)
    pub col: usize,
    /// Length of the match in characters
    pub length: usize,
    /// Matched text
    pub text: String,
}

/// Detected content item (URL, file path, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedItem {
    /// URL with position (url, col, row)
    Url(String, usize, usize),
    /// File path with position and optional line number (path, col, row, line_number)
    FilePath(String, usize, usize, Option<usize>),
    /// Git hash (7-40 chars) with position (hash, col, row)
    GitHash(String, usize, usize),
    /// IP address with position (ip, col, row)
    IpAddress(String, usize, usize),
    /// Email address with position (email, col, row)
    Email(String, usize, usize),
}

/// Regex match with position and captured groups
#[derive(Debug, Clone)]
pub struct RegexMatch {
    /// Row where match starts
    pub row: usize,
    /// Column where match starts
    pub col: usize,
    /// Row where match ends
    pub end_row: usize,
    /// Column where match ends
    pub end_col: usize,
    /// Length of match in characters
    pub length: usize,
    /// Matched text
    pub text: String,
    /// Capture groups (if any)
    pub captures: Vec<String>,
}

/// Options for regex search
#[derive(Debug, Clone)]
pub struct RegexSearchOptions {
    /// Case insensitive search
    pub case_insensitive: bool,
    /// Multiline mode (^ and $ match line boundaries)
    pub multiline: bool,
    /// Include scrollback in search
    pub include_scrollback: bool,
    /// Maximum number of matches to return (0 = unlimited)
    pub max_matches: usize,
    /// Search backwards from end
    pub reverse: bool,
}

impl Default for RegexSearchOptions {
    fn default() -> Self {
        Self {
            case_insensitive: false,
            multiline: true,
            include_scrollback: true,
            max_matches: 0,
            reverse: false,
        }
    }
}

use crate::terminal::Terminal;

impl Terminal {
    // === Feature 15: Regex Search in Scrollback ===

    /// Search for a regex pattern in terminal content
    ///
    /// Returns a list of matches with their positions and captured groups.
    pub fn search(
        &mut self,
        pattern: &str,
        options: RegexSearchOptions,
    ) -> Result<Vec<RegexMatch>, String> {
        let mut builder = regex::RegexBuilder::new(pattern);
        builder.case_insensitive(options.case_insensitive);
        builder.multi_line(options.multiline);

        let re = builder.build().map_err(|e| e.to_string())?;
        self.current_regex_pattern = Some(pattern.to_string());

        let mut all_content = Vec::new();
        let grid = self.active_grid();

        // Collect content based on options
        let start_row = if options.include_scrollback {
            -(grid.scrollback_len() as isize)
        } else {
            0
        };

        for row in start_row..(grid.rows() as isize) {
            let row_text = grid.row_text(row as usize);
            all_content.push((row, row_text));
        }

        let mut matches = Vec::new();
        for (row_idx, text) in all_content {
            for caps in re.captures_iter(&text) {
                if let Some(m) = caps.get(0) {
                    let mut captures = Vec::new();
                    for i in 0..caps.len() {
                        captures.push(
                            caps.get(i)
                                .map(|c| c.as_str().to_string())
                                .unwrap_or_default(),
                        );
                    }

                    let char_start = text[..m.start()].chars().count();
                    let char_len = m.as_str().chars().count();

                    matches.push(RegexMatch {
                        row: row_idx as usize,
                        col: char_start,
                        end_row: row_idx as usize,
                        end_col: char_start + char_len,
                        length: char_len,
                        text: m.as_str().to_string(),
                        captures,
                    });

                    if options.max_matches > 0 && matches.len() >= options.max_matches {
                        break;
                    }
                }
            }
            if options.max_matches > 0 && matches.len() >= options.max_matches {
                break;
            }
        }

        if options.reverse {
            matches.reverse();
        }

        self.regex_matches = matches.clone();
        Ok(matches)
    }

    /// Alias for search
    pub fn regex_search(
        &mut self,
        pattern: &str,
        options: RegexSearchOptions,
    ) -> Result<Vec<RegexMatch>, String> {
        self.search(pattern, options)
    }

    /// Get current search matches
    pub fn get_search_matches(&self) -> &[RegexMatch] {
        &self.regex_matches
    }

    /// Alias for get_search_matches
    pub fn get_regex_matches(&self) -> &[RegexMatch] {
        self.get_search_matches()
    }

    /// Get the current regex search pattern
    pub fn get_current_regex_pattern(&self) -> Option<String> {
        self.current_regex_pattern.clone()
    }

    /// Clear current search matches
    pub fn clear_search_matches(&mut self) {
        self.regex_matches.clear();
        self.current_regex_pattern = None;
    }

    /// Alias for clear_search_matches
    pub fn clear_regex_matches(&mut self) {
        self.clear_search_matches();
    }

    /// Find next regex match from position
    pub fn next_regex_match(&self, from_row: usize, from_col: usize) -> Option<RegexMatch> {
        self.regex_matches
            .iter()
            .find(|m| {
                if m.row > from_row {
                    true
                } else if m.row == from_row {
                    m.col > from_col
                } else {
                    false
                }
            })
            .cloned()
    }

    /// Find previous regex match from position
    pub fn prev_regex_match(&self, from_row: usize, from_col: usize) -> Option<RegexMatch> {
        self.regex_matches
            .iter()
            .rev()
            .find(|m| {
                if m.row < from_row {
                    true
                } else if m.row == from_row {
                    m.col < from_col
                } else {
                    false
                }
            })
            .cloned()
    }

    // === Search Methods ===

    /// Convert a byte offset within a string to a character (grapheme) offset.
    /// This is needed because `String::find()` returns byte offsets, but terminal
    /// columns are based on character positions.
    fn byte_offset_to_char_offset(s: &str, byte_offset: usize) -> usize {
        s[..byte_offset].chars().count()
    }

    /// Search for text in the visible screen area
    ///
    /// Returns a vector of SearchMatch results containing position and matched text.
    /// Row indices are 0-based, with 0 being the top row of the visible screen.
    ///
    /// # Arguments
    /// * `query` - The text to search for
    /// * `case_sensitive` - Whether the search should be case-sensitive
    pub fn search_text(&self, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }

        let grid = self.active_grid();
        let search_query = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);
                let search_text = if case_sensitive {
                    line_text.clone()
                } else {
                    line_text.to_lowercase()
                };

                // Use byte offsets for String::find(), but convert to char offsets for results
                let query_char_len = query.chars().count();
                let mut start_byte = 0;
                while let Some(pos) = search_text[start_byte..].find(&search_query) {
                    let match_byte_offset = start_byte + pos;
                    let char_column =
                        Self::byte_offset_to_char_offset(&search_text, match_byte_offset);

                    // Extract matched text using character iteration
                    let matched_text: String = line_text
                        .chars()
                        .skip(char_column)
                        .take(query_char_len)
                        .collect();

                    matches.push(SearchMatch {
                        row: row as isize,
                        col: char_column,
                        length: query_char_len,
                        text: matched_text,
                    });

                    // Advance by at least 1 byte (or the query length in bytes) to find next match
                    start_byte = match_byte_offset + search_query.len().max(1);
                }
            }
        }

        matches
    }

    /// Search for text in the scrollback buffer
    ///
    /// Returns matches with negative row indices (e.g., -1 is the most recent scrollback line).
    /// Row -1 is the line just above the visible screen.
    ///
    /// # Arguments
    /// * `query` - The text to search for
    /// * `case_sensitive` - Whether the search should be case-sensitive
    /// * `max_lines` - Maximum number of scrollback lines to search (None = search all)
    pub fn search_scrollback(
        &self,
        query: &str,
        case_sensitive: bool,
        max_lines: Option<usize>,
    ) -> Vec<SearchMatch> {
        let mut matches = Vec::new();
        if query.is_empty() {
            return matches;
        }

        let search_query = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        let scrollback_len = self.grid().scrollback_len();
        let lines_to_search = max_lines.unwrap_or(scrollback_len).min(scrollback_len);

        for i in 0..lines_to_search {
            if let Some(line) = self.grid().scrollback_line(i) {
                let line_text = crate::terminal::cells_to_text(line);
                let search_text = if case_sensitive {
                    line_text.clone()
                } else {
                    line_text.to_lowercase()
                };

                // Use byte offsets for String::find(), but convert to char offsets for results
                let query_char_len = query.chars().count();
                let mut start_byte = 0;
                while let Some(pos) = search_text[start_byte..].find(&search_query) {
                    let match_byte_offset = start_byte + pos;
                    let char_column =
                        Self::byte_offset_to_char_offset(&search_text, match_byte_offset);

                    // Extract matched text using character iteration
                    let matched_text: String = line_text
                        .chars()
                        .skip(char_column)
                        .take(query_char_len)
                        .collect();

                    matches.push(SearchMatch {
                        row: -((i + 1) as isize), // Negative indices for scrollback
                        col: char_column,
                        length: query_char_len,
                        text: matched_text,
                    });

                    // Advance by at least 1 byte (or the query length in bytes) to find next match
                    start_byte = match_byte_offset + search_query.len().max(1);
                }
            }
        }

        matches
    }

    // === Content Detection Methods ===

    /// Detect URLs in the visible screen
    ///
    /// Returns a vector of detected URLs with their positions.
    pub fn detect_urls(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        // Simple URL pattern: looks for http://, https://, ftp://, etc.
        let url_prefixes = ["http://", "https://", "ftp://", "ftps://"];

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);

                for prefix in &url_prefixes {
                    let mut start_col = 0;
                    while let Some(pos) = line_text[start_col..].to_lowercase().find(prefix) {
                        let col = start_col + pos;
                        // Find end of URL (space, newline, or end of line)
                        let end = line_text[col..]
                            .find(|c: char| c.is_whitespace())
                            .map(|p| col + p)
                            .unwrap_or(line_text.len());

                        if end > col {
                            let url = line_text[col..end].to_string();
                            items.push(DetectedItem::Url(url, col, row));
                        }
                        start_col = end.max(col + 1);
                    }
                }
            }
        }

        items
    }

    /// Detect file paths in the visible screen
    ///
    /// Returns a vector of detected file paths with their positions.
    /// Optionally includes line numbers if detected (e.g., "file.txt:123").
    pub fn detect_file_paths(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);

                // Simple detection: paths starting with / or ./ or ../
                let path_patterns = ["/", "./", "../"];

                for pattern in &path_patterns {
                    let mut start_col = 0;
                    while let Some(pos) = line_text[start_col..].find(pattern) {
                        let col = start_col + pos;
                        // Find end of path (whitespace or common delimiters)
                        let end = line_text[col..]
                            .find(|c: char| c.is_whitespace() || c == ':' || c == ',' || c == ')')
                            .map(|p| col + p)
                            .unwrap_or(line_text.len());

                        if end > col {
                            let path_str = line_text[col..end].to_string();

                            // Check for line number suffix (e.g., ":123")
                            let line_num = if end < line_text.len()
                                && line_text.chars().nth(end) == Some(':')
                            {
                                let num_start = end + 1;
                                let num_end = line_text[num_start..]
                                    .find(|c: char| !c.is_numeric())
                                    .map(|p| num_start + p)
                                    .unwrap_or(line_text.len());

                                if num_end > num_start {
                                    line_text[num_start..num_end].parse().ok()
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            items.push(DetectedItem::FilePath(path_str, col, row, line_num));
                        }
                        start_col = end.max(col + 1);
                    }
                }
            }
        }

        items
    }

    /// Detect semantic items (URLs, file paths, git hashes, IPs, emails)
    ///
    /// Returns all detected items in the visible screen.
    pub fn detect_semantic_items(&self) -> Vec<DetectedItem> {
        let mut items = Vec::new();
        let grid = self.active_grid();

        for row in 0..grid.rows() {
            if let Some(line) = grid.row(row) {
                let line_text = crate::terminal::cells_to_text(line);

                // Git hash pattern (40 hex chars)
                for (i, window) in line_text.as_bytes().windows(40).enumerate() {
                    if window.iter().all(|&b| b.is_ascii_hexdigit()) {
                        let hash = String::from_utf8_lossy(window).to_string();
                        items.push(DetectedItem::GitHash(hash, i, row));
                    }
                }

                // IP address pattern (simple v4)
                let ip_parts: Vec<&str> = line_text
                    .split(|c: char| !c.is_numeric() && c != '.')
                    .collect();
                for part in ip_parts.iter() {
                    let nums: Vec<&str> = part.split('.').collect();
                    if nums.len() == 4 && nums.iter().all(|n| n.parse::<u8>().is_ok()) {
                        if let Some(col) = line_text.find(part) {
                            items.push(DetectedItem::IpAddress(part.to_string(), col, row));
                        }
                    }
                }

                // Email pattern (simple)
                if let Some(at_pos) = line_text.find('@') {
                    // Find start of email
                    let start = line_text[..at_pos]
                        .rfind(|c: char| c.is_whitespace())
                        .map(|p| p + 1)
                        .unwrap_or(0);

                    // Find end of email
                    let end = line_text[at_pos..]
                        .find(|c: char| c.is_whitespace())
                        .map(|p| at_pos + p)
                        .unwrap_or(line_text.len());

                    if end > start && line_text[start..end].contains('@') {
                        items.push(DetectedItem::Email(
                            line_text[start..end].to_string(),
                            start,
                            row,
                        ));
                    }
                }
            }
        }

        // Also add URLs and file paths
        items.extend(self.detect_urls());
        items.extend(self.detect_file_paths());

        items
    }

    /// Get the URL at the given position
    pub fn get_url_at(&self, col: usize, row: usize) -> Option<String> {
        let urls = self.detect_urls();
        for item in urls {
            if let DetectedItem::Url(url, c, r) = item {
                if r == row && col >= c && col < c + url.len() {
                    return Some(url);
                }
            }
        }
        None
    }

    /// Find text in the visible buffer
    pub fn find_text(&self, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        self.search_text(query, case_sensitive)
    }

    /// Find next occurrence of text from a starting position
    pub fn find_next(
        &self,
        query: &str,
        from_col: usize,
        from_row: usize,
        case_sensitive: bool,
    ) -> Option<SearchMatch> {
        let matches = self.search_text(query, case_sensitive);
        matches.into_iter().find(|m| {
            if m.row as usize > from_row {
                true
            } else if m.row as usize == from_row {
                m.col > from_col
            } else {
                false
            }
        })
    }

    /// Find the matching bracket for the bracket at (col, row)
    pub fn find_matching_bracket(&self, col: usize, row: usize) -> Option<(usize, usize)> {
        let grid = self.active_grid();
        let cell = grid.get(col, row)?;
        let c = cell.c;

        let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
        let (open, close, forward) = if let Some(pair) = pairs.iter().find(|p| p.0 == c) {
            (pair.0, pair.1, true)
        } else if let Some(pair) = pairs.iter().find(|p| p.1 == c) {
            (pair.1, pair.0, false)
        } else {
            return None;
        };

        let mut depth = 0;
        if forward {
            for r in row..grid.rows() {
                let start_c = if r == row { col } else { 0 };
                if let Some(line) = grid.row(r) {
                    let line_text = crate::terminal::cells_to_text(line);
                    for (c_idx, ch) in line_text.chars().enumerate().skip(start_c) {
                        if ch == open {
                            depth += 1;
                        } else if ch == close {
                            depth -= 1;
                            if depth == 0 {
                                return Some((c_idx, r));
                            }
                        }
                    }
                }
            }
        } else {
            for r in (0..=row).rev() {
                let start_c = if r == row { col } else { grid.cols() - 1 };
                if let Some(line) = grid.row(r) {
                    let line_text = crate::terminal::cells_to_text(line);
                    let chars: Vec<char> = line_text.chars().collect();
                    for c_idx in (0..=start_c).rev() {
                        if let Some(&ch) = chars.get(c_idx) {
                            if ch == open {
                                depth += 1;
                            } else if ch == close {
                                depth -= 1;
                                if depth == 0 {
                                    return Some((c_idx, r));
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Get all hyperlinks in the current buffer
    pub fn get_all_hyperlinks(&self) -> Vec<HyperlinkInfo> {
        let mut links: std::collections::HashMap<u32, HyperlinkInfo> =
            std::collections::HashMap::new();
        let (_cols, rows) = self.size();
        let grid = self.active_grid();

        for row in 0..rows {
            if let Some(line) = grid.row(row) {
                for (col, cell) in line.iter().enumerate() {
                    if let Some(id) = cell.flags.hyperlink_id {
                        if let Some(url) = self.hyperlinks.get(&id) {
                            let entry = links.entry(id).or_insert_with(|| HyperlinkInfo {
                                url: url.clone(),
                                positions: Vec::new(),
                                id: Some(id.to_string()),
                            });
                            entry.positions.push((col, row));
                        }
                    }
                }
            }
        }

        links.into_values().collect()
    }
}
