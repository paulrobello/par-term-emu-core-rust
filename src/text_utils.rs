//! Text extraction and manipulation utilities

use crate::cell::Cell;
use crate::grid::Grid;

/// Default word characters for word boundary detection (iTerm2-compatible)
/// Matches iTerm2's default: slash, hyphen, plus, backslash, tilde, underscore, dot
pub const DEFAULT_WORD_CHARS: &str = "/-+\\~_.";

/// Check if a character is a word character
pub fn is_word_char(c: char, word_chars: Option<&str>) -> bool {
    c.is_alphanumeric() || word_chars.unwrap_or(DEFAULT_WORD_CHARS).contains(c)
}

/// Classify a grid cell for word membership.
///
/// A multi-char cell holds one grapheme cluster (base char + combining
/// marks), so the base character decides for the whole cluster.
fn cell_is_word(cell: &Cell, word_chars: Option<&str>) -> bool {
    is_word_char(cell.c(), word_chars)
}

/// Display columns a cell occupies (spacer cells occupy none of their own).
fn cell_display_width(cell: &Cell) -> usize {
    if cell.flags().wide_char_spacer() {
        0
    } else {
        cell.width().max(1)
    }
}

/// Resolve a display column to the index of the cell whose columns contain it.
///
/// A wide character's trailing spacer column resolves to the wide character.
/// Columns past the row's cells resolve to `None`.
fn cell_index_at_col(cells: &[Cell], col: usize) -> Option<usize> {
    let mut display_col = 0usize;
    for (i, cell) in cells.iter().enumerate() {
        let w = cell_display_width(cell);
        if col < display_col + w {
            return Some(i);
        }
        display_col += w;
    }
    None
}

/// Display column at which the cell at `idx` starts.
fn display_col_of(cells: &[Cell], idx: usize) -> usize {
    cells[..idx].iter().map(cell_display_width).sum()
}

/// Inclusive cell-index span `[start, end]` of the word containing cell `idx`.
///
/// Returns `None` when the cell at `idx` is not a word cell. Spacers belong
/// to the wide character that precedes them and never bound a word.
fn word_cell_span(cells: &[Cell], idx: usize, word_chars: Option<&str>) -> Option<(usize, usize)> {
    if !cell_is_word(&cells[idx], word_chars) {
        return None;
    }

    let mut start = idx;
    while start > 0 {
        let prev = start - 1;
        let prev = if cells[prev].flags().wide_char_spacer() {
            prev.checked_sub(1)?
        } else {
            prev
        };
        if !cell_is_word(&cells[prev], word_chars) {
            break;
        }
        start = prev;
    }

    let mut end = idx;
    loop {
        let mut next = end + 1;
        if next < cells.len() && cells[next].flags().wide_char_spacer() {
            next += 1;
        }
        if next >= cells.len() || !cell_is_word(&cells[next], word_chars) {
            break;
        }
        end = next;
    }

    Some((start, end))
}

/// Extract word at the given position
///
/// `col` is a display column; cells are walked directly so wide characters
/// and multi-char grapheme clusters (emoji + ZWJ, combining marks) map
/// columns correctly.
pub fn get_word_at(
    grid: &Grid,
    col: usize,
    row: usize,
    word_chars: Option<&str>,
) -> Option<String> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }
    let cells = grid.row(row)?;

    let idx = cell_index_at_col(cells, col)?;
    let (start, end) = word_cell_span(cells, idx, word_chars)?;

    let mut result = String::new();
    for cell in &cells[start..=end] {
        if !cell.flags().wide_char_spacer() {
            cell.push_grapheme(&mut result);
        }
    }
    Some(result)
}

/// Find word boundaries at position
///
/// `col` is a display column; the returned bounds are display columns
/// `(start_col, end_col)` where `end_col` is exclusive (the column just
/// past the last cell of the word).
pub fn select_word(
    grid: &Grid,
    col: usize,
    row: usize,
    word_chars: Option<&str>,
) -> Option<((usize, usize), (usize, usize))> {
    if row >= grid.rows() || col >= grid.cols() {
        return None;
    }
    let cells = grid.row(row)?;

    let idx = cell_index_at_col(cells, col)?;
    let (start, end) = word_cell_span(cells, idx, word_chars)?;

    let start_col = display_col_of(cells, start);
    let end_col = display_col_of(cells, end) + cell_display_width(&cells[end]);
    Some(((start_col, row), (end_col, row)))
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
    fn test_is_word_char_defaults() {
        assert!(is_word_char('a', None));
        assert!(is_word_char('Z', None));
        assert!(is_word_char('0', None));
        assert!(is_word_char('_', None));
        assert!(is_word_char('.', None));
        assert!(is_word_char('-', None));
        assert!(!is_word_char(' ', None));
        assert!(!is_word_char('(', None));
    }

    #[test]
    fn test_is_word_char_custom() {
        let custom = "@#";
        assert!(is_word_char('@', Some(custom)));
        assert!(is_word_char('#', Some(custom)));
        assert!(!is_word_char('.', Some(custom)));
    }

    #[test]
    fn test_select_word_boundaries() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "hello world".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_word(&grid, 2, 0, None);
        assert!(result.is_some());
        let ((start_col, start_row), (end_col, end_row)) = result.unwrap();
        assert_eq!(start_col, 0);
        assert_eq!(end_col, 5);
        assert_eq!(start_row, 0);
        assert_eq!(end_row, 0);
    }

    #[test]
    fn test_select_word_on_space() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "hello world".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        // Click on space
        let result = select_word(&grid, 5, 0, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_select_semantic_region_quotes() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "\"hello world\"".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 5, 0, "\"");
        assert_eq!(result, Some("hello world".to_string()));
    }

    #[test]
    fn test_select_semantic_region_parentheses() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "(test)".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 2, 0, "()");
        assert_eq!(result, Some("test".to_string()));
    }

    #[test]
    fn test_select_semantic_region_brackets() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "[array]".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 3, 0, "[]");
        assert_eq!(result, Some("array".to_string()));
    }

    #[test]
    fn test_select_semantic_region_curly() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "{data}".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 2, 0, "{}");
        assert_eq!(result, Some("data".to_string()));
    }

    #[test]
    fn test_select_semantic_region_nested() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "((inner))".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 4, 0, "()");
        assert_eq!(result, Some("inner".to_string()));
    }

    #[test]
    fn test_select_semantic_region_not_found() {
        use crate::cell::Cell;
        let mut grid = Grid::new(80, 24, 0);

        for (i, c) in "hello".chars().enumerate() {
            grid.set(i, 0, Cell::new(c));
        }

        let result = select_semantic_region(&grid, 2, 0, "\"");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_word_at_invalid_position() {
        let grid = Grid::new(80, 24, 0);
        assert!(get_word_at(&grid, 100, 0, None).is_none());
        assert!(get_word_at(&grid, 0, 100, None).is_none());
    }

    #[test]
    fn test_select_word_invalid_position() {
        let grid = Grid::new(80, 24, 0);
        assert!(select_word(&grid, 100, 0, None).is_none());
        assert!(select_word(&grid, 0, 100, None).is_none());
    }

    #[test]
    fn test_select_semantic_region_invalid_position() {
        let grid = Grid::new(80, 24, 0);
        assert!(select_semantic_region(&grid, 100, 0, "\"").is_none());
        assert!(select_semantic_region(&grid, 0, 100, "\"").is_none());
    }
}
