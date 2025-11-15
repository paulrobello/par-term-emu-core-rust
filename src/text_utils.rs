//! Text extraction and manipulation utilities

use crate::grid::Grid;
use unicode_width::UnicodeWidthChar;

/// Default word characters for word boundary detection
pub const DEFAULT_WORD_CHARS: &str = "-_.~:/?#[]@!$&'()*+,;=";

/// Check if a character is a word character
pub fn is_word_char(c: char, word_chars: Option<&str>) -> bool {
    c.is_alphanumeric() || word_chars.unwrap_or(DEFAULT_WORD_CHARS).contains(c)
}

/// Extract word at the given position
pub fn get_word_at(
    grid: &Grid,
    col: usize,
    row: usize,
    word_chars: Option<&str>,
) -> Option<String> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }

    let line = grid.row_text(row);
    if line.is_empty() {
        return None;
    }

    // Convert col to character index
    let mut char_col = 0;
    let mut byte_pos = 0;
    for (i, c) in line.char_indices() {
        if char_col == col {
            byte_pos = i;
            break;
        }
        char_col += c.width().unwrap_or(1);
        if char_col > col {
            return None; // Inside a wide character
        }
    }

    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }

    let char_idx = line[..byte_pos].chars().count();
    if char_idx >= chars.len() {
        return None;
    }

    let c = chars[char_idx];
    if !is_word_char(c, word_chars) {
        return None;
    }

    // Find start of word
    let mut start = char_idx;
    while start > 0 && is_word_char(chars[start - 1], word_chars) {
        start -= 1;
    }

    // Find end of word
    let mut end = char_idx + 1;
    while end < chars.len() && is_word_char(chars[end], word_chars) {
        end += 1;
    }

    Some(chars[start..end].iter().collect())
}

/// Check if a character is valid in a URL
fn is_url_char(c: char) -> bool {
    c.is_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c)
}

/// Extract URL at the given position
pub fn get_url_at(grid: &Grid, col: usize, row: usize) -> Option<String> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }

    let line = grid.row_text(row);
    if line.is_empty() {
        return None;
    }

    // Find URL schemes
    let schemes = [
        "http://", "https://", "ftp://", "file://", "mailto:", "ssh://",
    ];

    // Convert col to byte position
    let mut char_col = 0;
    let mut byte_pos = 0;
    for (i, c) in line.char_indices() {
        if char_col == col {
            byte_pos = i;
            break;
        }
        char_col += c.width().unwrap_or(1);
        if char_col > col {
            return None;
        }
    }

    // Search backwards and forwards for URL boundaries
    let chars: Vec<char> = line.chars().collect();
    let char_idx = line[..byte_pos].chars().count();

    if char_idx >= chars.len() {
        return None;
    }

    // Find potential URL start
    let mut start = char_idx;
    while start > 0 && is_url_char(chars[start - 1]) {
        start -= 1;
    }

    // Find potential URL end
    let mut end = char_idx + 1;
    while end < chars.len() && is_url_char(chars[end]) {
        end += 1;
    }

    let potential_url: String = chars[start..end].iter().collect();

    // Check if it contains a scheme
    for scheme in &schemes {
        if potential_url.contains(scheme) {
            // Find the actual start of the scheme
            if let Some(scheme_pos) = potential_url.find(scheme) {
                let url = &potential_url[scheme_pos..];
                return Some(url.to_string());
            }
        }
    }

    None
}

/// Get the full logical line following wrapping
pub fn get_line_unwrapped(grid: &Grid, row: usize) -> Option<String> {
    if row >= grid.rows() {
        return None;
    }

    let mut result = String::new();
    let mut current_row = row;

    // Go back to find the start of the logical line
    while current_row > 0 && grid.is_line_wrapped(current_row) {
        current_row -= 1;
    }

    // Now collect forward
    loop {
        let line = grid.row_text(current_row);
        result.push_str(&line);

        current_row += 1;
        if current_row >= grid.rows() || !grid.is_line_wrapped(current_row) {
            break;
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Find word boundaries at position
pub fn select_word(
    grid: &Grid,
    col: usize,
    row: usize,
    word_chars: Option<&str>,
) -> Option<((usize, usize), (usize, usize))> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }

    let line = grid.row_text(row);
    if line.is_empty() {
        return None;
    }

    // Convert col to character index
    let mut char_col = 0;
    let mut byte_pos = 0;
    for (i, c) in line.char_indices() {
        if char_col == col {
            byte_pos = i;
            break;
        }
        char_col += c.width().unwrap_or(1);
        if char_col > col {
            return None;
        }
    }

    let chars: Vec<char> = line.chars().collect();
    let char_idx = line[..byte_pos].chars().count();

    if char_idx >= chars.len() {
        return None;
    }

    let c = chars[char_idx];
    if !is_word_char(c, word_chars) {
        return None;
    }

    // Find start
    let mut start_idx = char_idx;
    while start_idx > 0 && is_word_char(chars[start_idx - 1], word_chars) {
        start_idx -= 1;
    }

    // Find end
    let mut end_idx = char_idx + 1;
    while end_idx < chars.len() && is_word_char(chars[end_idx], word_chars) {
        end_idx += 1;
    }

    // Convert character indices to column positions
    let start_col = chars[..start_idx]
        .iter()
        .map(|c| c.width().unwrap_or(1))
        .sum();
    let end_col = chars[..end_idx]
        .iter()
        .map(|c| c.width().unwrap_or(1))
        .sum();

    Some(((start_col, row), (end_col, row)))
}

/// Find matching bracket/parenthesis at position
///
/// Returns the position of the matching bracket, or None if:
/// - Not on a bracket character
/// - No matching bracket found
/// - Position is invalid
pub fn find_matching_bracket(grid: &Grid, col: usize, row: usize) -> Option<(usize, usize)> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }

    let line = grid.row_text(row);
    if line.is_empty() || col >= line.len() {
        return None;
    }

    // Get character at position
    let chars: Vec<char> = line.chars().collect();
    let char_idx = line[..col.min(line.len())].chars().count();
    if char_idx >= chars.len() {
        return None;
    }

    let ch = chars[char_idx];

    // Define bracket pairs
    let open_brackets = ['(', '[', '{', '<'];
    let close_brackets = [')', ']', '}', '>'];

    let (is_opening, bracket_idx) = if let Some(idx) = open_brackets.iter().position(|&b| b == ch) {
        (true, idx)
    } else if let Some(idx) = close_brackets.iter().position(|&b| b == ch) {
        (false, idx)
    } else {
        return None; // Not a bracket
    };

    let opening = open_brackets[bracket_idx];
    let closing = close_brackets[bracket_idx];

    if is_opening {
        // Search forward for closing bracket
        let mut depth = 1;
        let mut search_row = row;

        loop {
            let search_line = grid.row_text(search_row);
            let search_chars: Vec<char> = search_line.chars().collect();

            for (idx, &c) in search_chars.iter().enumerate().skip(if search_row == row {
                char_idx + 1
            } else {
                0
            }) {
                if c == opening {
                    depth += 1;
                } else if c == closing {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching bracket - convert char index to column
                        let match_col = search_chars[..idx]
                            .iter()
                            .map(|c| c.width().unwrap_or(1))
                            .sum();
                        return Some((match_col, search_row));
                    }
                }
            }

            search_row += 1;
            if search_row >= grid.rows() {
                break;
            }
        }
    } else {
        // Search backward for opening bracket
        let mut depth = 1;
        let mut search_row = row;

        loop {
            let search_line = grid.row_text(search_row);
            let search_chars: Vec<char> = search_line.chars().collect();

            let end_idx = if search_row == row {
                char_idx
            } else {
                search_chars.len()
            };

            for idx in (0..end_idx).rev() {
                let c = search_chars[idx];
                if c == closing {
                    depth += 1;
                } else if c == opening {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching bracket - convert char index to column
                        let match_col = search_chars[..idx]
                            .iter()
                            .map(|c| c.width().unwrap_or(1))
                            .sum();
                        return Some((match_col, search_row));
                    }
                }
            }

            if search_row == 0 {
                break;
            }
            search_row -= 1;
        }
    }

    None // No matching bracket found
}

/// Select text within semantic delimiters (quotes, brackets, etc.)
///
/// Finds and returns text between matching delimiters around the cursor position.
/// Supports: (), [], {}, <>, "", '', ``
///
/// Returns None if:
/// - Position is invalid
/// - Not inside delimiters
/// - Delimiters not found
pub fn select_semantic_region(
    grid: &Grid,
    col: usize,
    row: usize,
    delimiters: &str,
) -> Option<String> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }

    let line = grid.row_text(row);
    if line.is_empty() {
        return None;
    }

    let chars: Vec<char> = line.chars().collect();
    let char_idx = line[..col.min(line.len())].chars().count();
    if char_idx >= chars.len() {
        return None;
    }

    // Define delimiter pairs
    let pairs = [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('<', '>'),
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
    ];

    // Filter pairs based on provided delimiters
    let active_pairs: Vec<(char, char)> = pairs
        .iter()
        .filter(|(open, close)| delimiters.contains(*open) || delimiters.contains(*close))
        .copied()
        .collect();

    if active_pairs.is_empty() {
        return None;
    }

    // Try each delimiter pair
    for (open_delim, close_delim) in active_pairs {
        let is_symmetric = open_delim == close_delim;

        // Search backward for opening delimiter
        let mut start_idx = None;
        let mut depth = 0;

        for idx in (0..char_idx).rev() {
            let c = chars[idx];
            if is_symmetric {
                // For symmetric delimiters like quotes, just find the previous one
                if c == open_delim {
                    start_idx = Some(idx);
                    break;
                }
            } else {
                // For asymmetric delimiters, track nesting depth
                if c == close_delim {
                    depth += 1;
                } else if c == open_delim {
                    if depth == 0 {
                        start_idx = Some(idx);
                        break;
                    }
                    depth -= 1;
                }
            }
        }

        if let Some(start) = start_idx {
            // Search forward for closing delimiter
            depth = 0;
            for idx in (char_idx + 1)..chars.len() {
                let c = chars[idx];
                if is_symmetric {
                    if c == close_delim {
                        // Found closing delimiter - extract content
                        let content: String = chars[(start + 1)..idx].iter().collect();
                        return Some(content);
                    }
                } else if c == open_delim {
                    depth += 1;
                } else if c == close_delim {
                    if depth == 0 {
                        // Found closing delimiter - extract content
                        let content: String = chars[(start + 1)..idx].iter().collect();
                        return Some(content);
                    }
                    depth -= 1;
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    #[test]
    fn test_get_word_at() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);
        grid.set(0, 0, Cell::new('h'));
        grid.set(1, 0, Cell::new('e'));
        grid.set(2, 0, Cell::new('l'));
        grid.set(3, 0, Cell::new('l'));
        grid.set(4, 0, Cell::new('o'));
        grid.set(5, 0, Cell::new(' '));
        grid.set(6, 0, Cell::new('w'));
        grid.set(7, 0, Cell::new('o'));
        grid.set(8, 0, Cell::new('r'));
        grid.set(9, 0, Cell::new('l'));
        grid.set(10, 0, Cell::new('d'));

        assert_eq!(get_word_at(&grid, 2, 0, None), Some("hello".to_string()));
        assert_eq!(get_word_at(&grid, 8, 0, None), Some("world".to_string()));
        assert_eq!(get_word_at(&grid, 5, 0, None), None); // Space
    }

    #[test]
    fn test_get_url_at() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);
        let url = "https://example.com/path";
        for (i, c) in url.chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = get_url_at(&grid, 10, 0);
        assert!(result.is_some());
        assert!(result.unwrap().starts_with("https://"));
    }
}
