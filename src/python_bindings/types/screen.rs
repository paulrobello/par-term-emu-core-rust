//! Screen snapshot, cell attributes, diffing, and scrollback types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

use super::LineCellData;
use crate::python_bindings::enums::{PyCursorStyle, PyUnderlineStyle};

/// Cell attributes
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "Attributes", from_py_object)]
#[derive(Clone)]
pub struct PyAttributes {
    /// Bold attribute (SGR 1)
    pub bold: bool,
    /// Dim/faint attribute (SGR 2)
    pub dim: bool,
    /// Italic attribute (SGR 3)
    pub italic: bool,
    /// Underline attribute (SGR 4)
    pub underline: bool,
    /// Blink attribute (SGR 5)
    pub blink: bool,
    /// Reverse video attribute (SGR 7)
    pub reverse: bool,
    /// Hidden/concealed attribute (SGR 8)
    pub hidden: bool,
    /// Strikethrough attribute (SGR 9)
    pub strikethrough: bool,
    /// Underline style (curl, dotted, dashed, ...)
    pub underline_style: PyUnderlineStyle,
    /// Whether the cell holds the first half of a double-width character
    pub wide_char: bool,
    /// Whether the cell is the spacer following a double-width character
    pub wide_char_spacer: bool,
    /// Hyperlink ID for OSC 8 links, if the cell is a link
    pub hyperlink_id: Option<u32>,
}

impl From<&crate::cell::Cell> for PyAttributes {
    fn from(cell: &crate::cell::Cell) -> Self {
        PyAttributes {
            bold: cell.flags.bold(),
            dim: cell.flags.dim(),
            italic: cell.flags.italic(),
            underline: cell.flags.underline(),
            blink: cell.flags.blink(),
            reverse: cell.flags.reverse(),
            hidden: cell.flags.hidden(),
            strikethrough: cell.flags.strikethrough(),
            underline_style: cell.flags.underline_style.into(),
            wide_char: cell.flags.wide_char(),
            wide_char_spacer: cell.flags.wide_char_spacer(),
            hyperlink_id: cell.flags.hyperlink_id.map(|nz| nz.get()),
        }
    }
}

impl Default for PyAttributes {
    fn default() -> Self {
        Self {
            bold: false,
            dim: false,
            italic: false,
            underline: false,
            blink: false,
            reverse: false,
            hidden: false,
            strikethrough: false,
            underline_style: PyUnderlineStyle::None,
            wide_char: false,
            wide_char_spacer: false,
            hyperlink_id: None,
        }
    }
}

#[pymethods]
impl PyAttributes {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Attributes(bold={}, italic={}, underline={}, underline_style={:?})",
            self.bold, self.italic, self.underline, self.underline_style
        ))
    }
}

/// Atomic snapshot of terminal screen state for race-free rendering
impl From<crate::terminal::replay_snapshot::TerminalSnapshot> for PyScreenSnapshot {
    fn from(snap: crate::terminal::replay_snapshot::TerminalSnapshot) -> Self {
        let active_grid = if snap.alt_screen_active {
            &snap.alt_grid
        } else {
            &snap.grid
        };

        let mut lines = Vec::with_capacity(snap.rows);
        for row in 0..snap.rows {
            let mut line = Vec::with_capacity(snap.cols);
            for col in 0..snap.cols {
                if let Some(cell) = active_grid.cells.get(row * snap.cols + col) {
                    let fg = cell.fg.to_rgb();
                    let bg = cell.bg.to_rgb();
                    line.push((cell.get_grapheme(), fg, bg, cell.into()));
                }
            }
            lines.push(line);
        }

        PyScreenSnapshot {
            lines,
            wrapped_lines: active_grid.wrapped.clone(),
            cursor_pos: (snap.cursor.col, snap.cursor.row),
            cursor_visible: snap.cursor.visible,
            cursor_style: snap.cursor.style().into(),
            is_alt_screen: snap.alt_screen_active,
            generation: 0, // generation not tracked in snapshots
            size: (snap.cols, snap.rows),
        }
    }
}

///
/// Captures all lines, cursor state, and screen identity at a single point in time.
/// This immutable snapshot prevents race conditions where alternate screen switches
/// happen between individual line render calls.
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ScreenSnapshot")]
pub struct PyScreenSnapshot {
    /// All screen lines captured atomically
    /// Format: Vec<Vec<(String, fg_rgb, bg_rgb, attributes)>>
    pub lines: Vec<LineCellData>,

    /// Wrapped state for each line (true = line continues to next row)
    pub wrapped_lines: Vec<bool>,

    /// Cursor position at snapshot time (col, row)
    pub cursor_pos: (usize, usize),

    /// Cursor visibility at snapshot time
    pub cursor_visible: bool,

    /// Cursor style at snapshot time
    pub cursor_style: PyCursorStyle,

    /// Which screen buffer was active (true = alternate)
    pub is_alt_screen: bool,

    /// Generation counter at snapshot time
    pub generation: u64,

    /// Terminal dimensions at snapshot time (cols, rows)
    pub size: (usize, usize),
}

#[pymethods]
impl PyScreenSnapshot {
    /// Get line cells for a specific row from snapshot
    ///
    /// Filters control characters (< 32, except space and tab) and replaces them with space.
    /// This optimization moves control character filtering from Python to compiled Rust code.
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes),
    ///     or empty list if row is out of bounds
    fn get_line(&self, row: usize) -> LineCellData {
        if row < self.lines.len() {
            // Clone and filter control characters in one pass
            self.lines[row]
                .iter()
                .map(|(c, fg, bg, attrs)| {
                    // Filter out control characters (< 32) except space and tab
                    // Check the first character of the grapheme string
                    let first_char = c.chars().next().unwrap_or(' ');
                    let filtered_char =
                        if (first_char as u32) < 32 && first_char != ' ' && first_char != '\t' {
                            " ".to_string() // Replace control chars with space
                        } else {
                            c.clone()
                        };
                    (filtered_char, *fg, *bg, attrs.clone())
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ScreenSnapshot(size={}x{}, gen={}, alt={})",
            self.size.0, self.size.1, self.generation, self.is_alt_screen
        ))
    }
}

/// Scrollback statistics
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ScrollbackStats", from_py_object)]
#[derive(Clone)]
pub struct PyScrollbackStats {
    /// Total number of scrollback lines
    pub total_lines: usize,
    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
    /// Whether the scrollback buffer has wrapped (cycled)
    pub has_wrapped: bool,
}

#[pymethods]
impl PyScrollbackStats {
    fn __repr__(&self) -> String {
        format!(
            "ScrollbackStats(total_lines={}, memory_bytes={}, has_wrapped={})",
            self.total_lines, self.memory_bytes, self.has_wrapped
        )
    }
}

/// Bookmark
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "Bookmark", from_py_object)]
#[derive(Clone)]
pub struct PyBookmark {
    /// Bookmark ID
    pub id: usize,
    /// Row index (negative for scrollback, 0+ for visible screen)
    pub row: isize,
    /// Bookmark label
    pub label: String,
}

#[pymethods]
impl PyBookmark {
    fn __repr__(&self) -> String {
        format!(
            "Bookmark(id={}, row={}, label={:?})",
            self.id, self.row, self.label
        )
    }
}

/// Joined lines result
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "JoinedLines", from_py_object)]
#[derive(Clone)]
pub struct PyJoinedLines {
    /// The joined text of the wrapped lines
    pub text: String,
    /// First row of the logical line
    pub start_row: usize,
    /// Last row of the logical line
    pub end_row: usize,
    /// Number of physical rows joined
    pub lines_joined: usize,
}

#[pymethods]
impl PyJoinedLines {
    fn __repr__(&self) -> String {
        format!(
            "JoinedLines(rows={}-{}, lines={}, len={})",
            self.start_row,
            self.end_row,
            self.lines_joined,
            self.text.len()
        )
    }
}

/// Damage region
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "DamageRegion", from_py_object)]
#[derive(Clone)]
pub struct PyDamageRegion {
    /// Left column of the damaged region (inclusive)
    pub left: usize,
    /// Top row of the damaged region (inclusive)
    pub top: usize,
    /// Right column of the damaged region (exclusive)
    pub right: usize,
    /// Bottom row of the damaged region (exclusive)
    pub bottom: usize,
}

#[pymethods]
impl PyDamageRegion {
    fn __repr__(&self) -> String {
        format!(
            "DamageRegion(left={}, top={}, right={}, bottom={})",
            self.left, self.top, self.right, self.bottom
        )
    }
}

impl From<&crate::terminal::DamageRegion> for PyDamageRegion {
    fn from(region: &crate::terminal::DamageRegion) -> Self {
        PyDamageRegion {
            left: region.left,
            top: region.top,
            right: region.right,
            bottom: region.bottom,
        }
    }
}

/// Rendering hint
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "RenderingHint", from_py_object)]
#[derive(Clone)]
pub struct PyRenderingHint {
    /// The dirty region that needs redrawing
    pub damage: PyDamageRegion,
    /// Z-layer the content should be drawn on (e.g. "base", "overlay")
    pub layer: String,
    /// Animation hint for the renderer (e.g. "none", "blink")
    pub animation: String,
    /// Update priority (0-255, higher redraws sooner)
    pub priority: u8,
}

#[pymethods]
impl PyRenderingHint {
    fn __repr__(&self) -> String {
        format!(
            "RenderingHint(layer={}, animation={}, priority={})",
            self.layer, self.animation, self.priority
        )
    }
}

impl From<&crate::terminal::RenderingHint> for PyRenderingHint {
    fn from(hint: &crate::terminal::RenderingHint) -> Self {
        use crate::terminal::{AnimationHint, ZLayer};

        let layer = match hint.layer {
            ZLayer::Background => "background",
            ZLayer::Normal => "normal",
            ZLayer::Overlay => "overlay",
            ZLayer::Cursor => "cursor",
        }
        .to_string();

        let animation = match hint.animation {
            AnimationHint::None => "none",
            AnimationHint::SmoothScroll => "smoothscroll",
            AnimationHint::Fade => "fade",
            AnimationHint::CursorBlink => "cursorblink",
        }
        .to_string();

        PyRenderingHint {
            damage: PyDamageRegion::from(&hint.damage),
            layer,
            animation,
            priority: hint.priority as u8,
        }
    }
}

/// Line diff
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "LineDiff", from_py_object)]
#[derive(Clone)]
pub struct PyLineDiff {
    /// Change kind: "added", "removed", or "modified"
    pub change_type: String,
    /// Row the line had in the old snapshot (None for added lines)
    pub old_row: Option<usize>,
    /// Row the line has in the new snapshot (None for removed lines)
    pub new_row: Option<usize>,
    /// Previous line content (None for added lines)
    pub old_content: Option<String>,
    /// Current line content (None for removed lines)
    pub new_content: Option<String>,
}

#[pymethods]
impl PyLineDiff {
    fn __repr__(&self) -> String {
        format!(
            "LineDiff(type={}, old_row={:?}, new_row={:?})",
            self.change_type, self.old_row, self.new_row
        )
    }
}

impl From<&crate::terminal::LineDiff> for PyLineDiff {
    fn from(diff: &crate::terminal::LineDiff) -> Self {
        use crate::terminal::DiffChangeType;

        let change_type = match diff.change_type {
            DiffChangeType::Added => "added",
            DiffChangeType::Removed => "removed",
            DiffChangeType::Modified => "modified",
            DiffChangeType::Unchanged => "unchanged",
        }
        .to_string();

        PyLineDiff {
            change_type,
            old_row: diff.old_row,
            new_row: diff.new_row,
            old_content: diff.old_content.clone(),
            new_content: diff.new_content.clone(),
        }
    }
}

/// Snapshot diff
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "SnapshotDiff", from_py_object)]
#[derive(Clone)]
pub struct PySnapshotDiff {
    /// Per-line differences
    pub diffs: Vec<PyLineDiff>,
    /// Number of added lines
    pub added: usize,
    /// Number of removed lines
    pub removed: usize,
    /// Number of modified lines
    pub modified: usize,
    /// Number of unchanged lines
    pub unchanged: usize,
}

#[pymethods]
impl PySnapshotDiff {
    fn __repr__(&self) -> String {
        format!(
            "SnapshotDiff(added={}, removed={}, modified={}, unchanged={})",
            self.added, self.removed, self.modified, self.unchanged
        )
    }
}

impl From<&crate::terminal::SnapshotDiff> for PySnapshotDiff {
    fn from(diff: &crate::terminal::SnapshotDiff) -> Self {
        PySnapshotDiff {
            diffs: diff.diffs.iter().map(PyLineDiff::from).collect(),
            added: diff.added,
            removed: diff.removed,
            modified: diff.modified,
            unchanged: diff.unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::python_bindings::enums::{PyCursorStyle, PyUnderlineStyle};

    #[test]
    fn test_pyattributes_default() {
        let attrs = PyAttributes::default();

        assert!(!attrs.bold);
        assert!(!attrs.dim);
        assert!(!attrs.italic);
        assert!(!attrs.underline);
        assert!(!attrs.blink);
        assert!(!attrs.reverse);
        assert!(!attrs.hidden);
        assert!(!attrs.strikethrough);
        assert!(matches!(attrs.underline_style, PyUnderlineStyle::None));
        assert!(!attrs.wide_char);
        assert!(!attrs.wide_char_spacer);
        assert_eq!(attrs.hyperlink_id, None);
    }

    #[test]
    fn test_pyattributes_repr() {
        let attrs = PyAttributes {
            bold: true,
            italic: true,
            underline: true,
            underline_style: PyUnderlineStyle::Straight,
            ..Default::default()
        };

        let repr = attrs.__repr__().unwrap();
        assert!(repr.contains("bold=true"));
        assert!(repr.contains("italic=true"));
        assert!(repr.contains("underline=true"));
        assert!(repr.contains("Straight"));
    }

    #[test]
    fn test_pyattributes_repr_all_false() {
        let attrs = PyAttributes::default();
        let repr = attrs.__repr__().unwrap();

        assert!(repr.contains("bold=false"));
        assert!(repr.contains("italic=false"));
        assert!(repr.contains("underline=false"));
    }

    #[test]
    fn test_pyattributes_clone() {
        let attrs1 = PyAttributes {
            bold: true,
            italic: true,
            hyperlink_id: Some(42),
            ..Default::default()
        };

        let attrs2 = attrs1.clone();

        assert_eq!(attrs1.bold, attrs2.bold);
        assert_eq!(attrs1.italic, attrs2.italic);
        assert_eq!(attrs1.hyperlink_id, attrs2.hyperlink_id);
    }

    #[test]
    fn test_pyattributes_with_hyperlink() {
        let attrs = PyAttributes {
            hyperlink_id: Some(123),
            ..Default::default()
        };

        assert_eq!(attrs.hyperlink_id, Some(123));
    }

    #[test]
    fn test_pyattributes_all_flags() {
        let attrs = PyAttributes {
            bold: true,
            dim: true,
            italic: true,
            underline: true,
            blink: true,
            reverse: true,
            hidden: true,
            strikethrough: true,
            wide_char: true,
            wide_char_spacer: true,
            underline_style: PyUnderlineStyle::Curly,
            hyperlink_id: Some(99),
        };

        assert!(attrs.bold);
        assert!(attrs.dim);
        assert!(attrs.italic);
        assert!(attrs.underline);
        assert!(attrs.blink);
        assert!(attrs.reverse);
        assert!(attrs.hidden);
        assert!(attrs.strikethrough);
        assert!(attrs.wide_char);
        assert!(attrs.wide_char_spacer);
        assert!(matches!(attrs.underline_style, PyUnderlineStyle::Curly));
        assert_eq!(attrs.hyperlink_id, Some(99));
    }

    #[test]
    fn test_pyscreensnapshot_get_line_valid_row() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "H".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ),
                (
                    "i".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ),
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);
        assert_eq!(line.len(), 2);
        assert_eq!(line[0].0, "H");
        assert_eq!(line[1].0, "i");
    }

    #[test]
    fn test_pyscreensnapshot_get_line_out_of_bounds() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![(
                "A".to_string(),
                (255, 255, 255),
                (0, 0, 0),
                PyAttributes::default(),
            )]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(5); // Row 5 doesn't exist
        assert_eq!(line.len(), 0);
    }

    #[test]
    fn test_pyscreensnapshot_get_line_filters_control_chars() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Control char
                (
                    "A".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Regular char
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // ESC
                (
                    " ".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Space (allowed)
                (
                    "\t".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Tab (allowed)
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);
        assert_eq!(line.len(), 5);
        assert_eq!(line[0].0, " "); // Control char replaced with space
        assert_eq!(line[1].0, "A"); // Regular char unchanged
        assert_eq!(line[2].0, " "); // ESC replaced with space
        assert_eq!(line[3].0, " "); // Space unchanged
        assert_eq!(line[4].0, "\t"); // Tab unchanged
    }

    #[test]
    fn test_pyscreensnapshot_repr() {
        let snapshot = PyScreenSnapshot {
            lines: vec![],
            wrapped_lines: vec![],
            cursor_pos: (10, 5),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: true,
            generation: 42,
            size: (80, 24),
        };

        let repr = snapshot.__repr__().unwrap();
        assert!(repr.contains("80x24"));
        assert!(repr.contains("gen=42"));
        assert!(repr.contains("alt=true"));
    }

    #[test]
    fn test_pyscreensnapshot_repr_not_alt_screen() {
        let snapshot = PyScreenSnapshot {
            lines: vec![],
            wrapped_lines: vec![],
            cursor_pos: (0, 0),
            cursor_visible: false,
            cursor_style: PyCursorStyle::BlinkingBlock,
            is_alt_screen: false,
            generation: 100,
            size: (120, 30),
        };

        let repr = snapshot.__repr__().unwrap();
        assert!(repr.contains("120x30"));
        assert!(repr.contains("gen=100"));
        assert!(repr.contains("alt=false"));
    }

    #[test]
    fn test_line_cell_data_type_alias() {
        // Test that the LineCellData type alias works correctly
        let cell_data: LineCellData = vec![
            (
                "A".to_string(),
                (255, 0, 0),
                (0, 0, 0),
                PyAttributes::default(),
            ),
            (
                "B".to_string(),
                (0, 255, 0),
                (0, 0, 0),
                PyAttributes::default(),
            ),
        ];

        assert_eq!(cell_data.len(), 2);
        assert_eq!(cell_data[0].0, "A");
        assert_eq!(cell_data[0].1, (255, 0, 0)); // Red
        assert_eq!(cell_data[1].0, "B");
        assert_eq!(cell_data[1].1, (0, 255, 0)); // Green
    }

    #[test]
    fn test_pyscreensnapshot_fields() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![]],
            wrapped_lines: vec![true, false],
            cursor_pos: (15, 10),
            cursor_visible: false,
            cursor_style: PyCursorStyle::BlinkingUnderline,
            is_alt_screen: true,
            generation: 999,
            size: (100, 50),
        };

        assert_eq!(snapshot.cursor_pos, (15, 10));
        assert!(!snapshot.cursor_visible);
        assert!(matches!(
            snapshot.cursor_style,
            PyCursorStyle::BlinkingUnderline
        ));
        assert!(snapshot.is_alt_screen);
        assert_eq!(snapshot.generation, 999);
        assert_eq!(snapshot.size, (100, 50));
        assert_eq!(snapshot.wrapped_lines.len(), 2);
        assert!(snapshot.wrapped_lines[0]);
        assert!(!snapshot.wrapped_lines[1]);
    }

    #[test]
    fn test_control_character_filtering_edge_cases() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![
                (
                    "\x00".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // NULL
                (
                    "\x1F".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Unit separator
                (
                    " ".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // Space (32)
                (
                    "!".to_string(),
                    (255, 255, 255),
                    (0, 0, 0),
                    PyAttributes::default(),
                ), // "!" (33)
            ]],
            wrapped_lines: vec![false],
            cursor_pos: (0, 0),
            cursor_visible: true,
            cursor_style: PyCursorStyle::SteadyBlock,
            is_alt_screen: false,
            generation: 1,
            size: (80, 24),
        };

        let line = snapshot.get_line(0);

        // Control chars (< 32) should be replaced with space
        assert_eq!(line[0].0, " "); // NULL -> space
        assert_eq!(line[1].0, " "); // Unit separator -> space

        // Space and above should be unchanged
        assert_eq!(line[2].0, " "); // Space unchanged
        assert_eq!(line[3].0, "!"); // "!" unchanged
    }
}
