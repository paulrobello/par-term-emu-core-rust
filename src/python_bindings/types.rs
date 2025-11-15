//! Python data types and structures for the terminal API
//!
//! This module contains the main Python-facing data structures:
//! - PyAttributes: Cell text attributes (bold, italic, etc.)
//! - PyScreenSnapshot: Atomic snapshot of terminal screen state
//! - PyShellIntegration: Shell integration (OSC 133) state
//! - PyGraphic: Sixel graphics representation
//! - LineCellData: Type alias for row cell data

use pyo3::prelude::*;

use super::enums::{PyCursorStyle, PyUnderlineStyle};

/// Type alias for a row of cell data returned by get_line_cells
/// Tuple contains: (character, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes)
pub type LineCellData = Vec<(char, (u8, u8, u8), (u8, u8, u8), PyAttributes)>;

/// Cell attributes
#[pyclass(name = "Attributes")]
#[derive(Clone)]
pub struct PyAttributes {
    #[pyo3(get)]
    pub bold: bool,
    #[pyo3(get)]
    pub dim: bool,
    #[pyo3(get)]
    pub italic: bool,
    #[pyo3(get)]
    pub underline: bool,
    #[pyo3(get)]
    pub blink: bool,
    #[pyo3(get)]
    pub reverse: bool,
    #[pyo3(get)]
    pub hidden: bool,
    #[pyo3(get)]
    pub strikethrough: bool,
    #[pyo3(get)]
    pub underline_style: PyUnderlineStyle,
    #[pyo3(get)]
    pub wide_char: bool,
    #[pyo3(get)]
    pub wide_char_spacer: bool,
    #[pyo3(get)]
    pub hyperlink_id: Option<u32>,
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
///
/// Captures all lines, cursor state, and screen identity at a single point in time.
/// This immutable snapshot prevents race conditions where alternate screen switches
/// happen between individual line render calls.
#[pyclass(name = "ScreenSnapshot")]
#[allow(clippy::type_complexity)]
pub struct PyScreenSnapshot {
    /// All screen lines captured atomically
    /// Format: Vec<Vec<(char, fg_rgb, bg_rgb, attributes)>>
    #[pyo3(get)]
    pub lines: Vec<Vec<(char, (u8, u8, u8), (u8, u8, u8), PyAttributes)>>,

    /// Wrapped state for each line (true = line continues to next row)
    #[pyo3(get)]
    pub wrapped_lines: Vec<bool>,

    /// Cursor position at snapshot time (col, row)
    #[pyo3(get)]
    pub cursor_pos: (usize, usize),

    /// Cursor visibility at snapshot time
    #[pyo3(get)]
    pub cursor_visible: bool,

    /// Cursor style at snapshot time
    #[pyo3(get)]
    pub cursor_style: PyCursorStyle,

    /// Which screen buffer was active (true = alternate)
    #[pyo3(get)]
    pub is_alt_screen: bool,

    /// Generation counter at snapshot time
    #[pyo3(get)]
    pub generation: u64,

    /// Terminal dimensions at snapshot time (cols, rows)
    #[pyo3(get)]
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
    #[allow(clippy::type_complexity)]
    fn get_line(&self, row: usize) -> Vec<(char, (u8, u8, u8), (u8, u8, u8), PyAttributes)> {
        if row < self.lines.len() {
            // Clone and filter control characters in one pass
            self.lines[row]
                .iter()
                .map(|(c, fg, bg, attrs)| {
                    // Filter out control characters (< 32) except space and tab
                    // Space is actually 32, so it won't be < 32, but we check it for clarity
                    // Tab is 9 (0x09)
                    let filtered_char = if (*c as u32) < 32 && *c != ' ' && *c != '\t' {
                        ' ' // Replace control chars with space
                    } else {
                        *c
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

/// Shell integration state
#[pyclass(name = "ShellIntegration")]
#[derive(Clone)]
pub struct PyShellIntegration {
    #[pyo3(get)]
    pub in_prompt: bool,
    #[pyo3(get)]
    pub in_command_input: bool,
    #[pyo3(get)]
    pub in_command_output: bool,
    #[pyo3(get)]
    pub current_command: Option<String>,
    #[pyo3(get)]
    pub last_exit_code: Option<i32>,
    #[pyo3(get)]
    pub cwd: Option<String>,
}

#[pymethods]
impl PyShellIntegration {
    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "ShellIntegration(in_prompt={}, in_command_input={}, in_command_output={})",
            self.in_prompt, self.in_command_input, self.in_command_output
        ))
    }
}

/// Sixel graphic representation
#[pyclass(name = "Graphic")]
#[derive(Clone)]
pub struct PyGraphic {
    #[pyo3(get)]
    pub position: (usize, usize),
    #[pyo3(get)]
    pub width: usize,
    #[pyo3(get)]
    pub height: usize,
    pixels: Vec<u8>,
}

#[pymethods]
impl PyGraphic {
    /// Get pixel color at (x, y) coordinates
    ///
    /// Args:
    ///     x: X coordinate (0-based)
    ///     y: Y coordinate (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b, a) values, or None if out of bounds
    fn get_pixel(&self, x: usize, y: usize) -> Option<(u8, u8, u8, u8)> {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 4;
            Some((
                self.pixels[idx],
                self.pixels[idx + 1],
                self.pixels[idx + 2],
                self.pixels[idx + 3],
            ))
        } else {
            None
        }
    }

    /// Get raw pixel data as bytes (RGBA format)
    ///
    /// Returns:
    ///     Bytes containing RGBA pixel data in row-major order
    fn pixels(&self) -> Vec<u8> {
        self.pixels.clone()
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Graphic(position=({},{}), size={}x{})",
            self.position.0, self.position.1, self.width, self.height
        ))
    }
}

impl From<&crate::sixel::SixelGraphic> for PyGraphic {
    fn from(graphic: &crate::sixel::SixelGraphic) -> Self {
        Self {
            position: graphic.position,
            width: graphic.width,
            height: graphic.height,
            pixels: graphic.pixels.clone(),
        }
    }
}
