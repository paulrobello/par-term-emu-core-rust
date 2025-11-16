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

#[cfg(test)]
mod tests {
    use super::*;

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
                ('H', (255, 255, 255), (0, 0, 0), PyAttributes::default()),
                ('i', (255, 255, 255), (0, 0, 0), PyAttributes::default()),
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
        assert_eq!(line[0].0, 'H');
        assert_eq!(line[1].0, 'i');
    }

    #[test]
    fn test_pyscreensnapshot_get_line_out_of_bounds() {
        let snapshot = PyScreenSnapshot {
            lines: vec![vec![(
                'A',
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
                ('\x01', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // Control char
                ('A', (255, 255, 255), (0, 0, 0), PyAttributes::default()),    // Regular char
                ('\x1B', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // ESC
                (' ', (255, 255, 255), (0, 0, 0), PyAttributes::default()),    // Space (allowed)
                ('\t', (255, 255, 255), (0, 0, 0), PyAttributes::default()),   // Tab (allowed)
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
        assert_eq!(line[0].0, ' '); // Control char replaced with space
        assert_eq!(line[1].0, 'A'); // Regular char unchanged
        assert_eq!(line[2].0, ' '); // ESC replaced with space
        assert_eq!(line[3].0, ' '); // Space unchanged
        assert_eq!(line[4].0, '\t'); // Tab unchanged
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
    fn test_pyshellintegration_repr() {
        let shell_int = PyShellIntegration {
            in_prompt: true,
            in_command_input: false,
            in_command_output: false,
            current_command: Some("ls -la".to_string()),
            last_exit_code: Some(0),
            cwd: Some("/home/user".to_string()),
        };

        let repr = shell_int.__repr__().unwrap();
        assert!(repr.contains("in_prompt=true"));
        assert!(repr.contains("in_command_input=false"));
        assert!(repr.contains("in_command_output=false"));
    }

    #[test]
    fn test_pyshellintegration_all_states() {
        let shell_int = PyShellIntegration {
            in_prompt: false,
            in_command_input: true,
            in_command_output: false,
            current_command: None,
            last_exit_code: None,
            cwd: None,
        };

        assert!(!shell_int.in_prompt);
        assert!(shell_int.in_command_input);
        assert!(!shell_int.in_command_output);
        assert_eq!(shell_int.current_command, None);
        assert_eq!(shell_int.last_exit_code, None);
        assert_eq!(shell_int.cwd, None);
    }

    #[test]
    fn test_pyshellintegration_clone() {
        let shell_int1 = PyShellIntegration {
            in_prompt: true,
            in_command_input: true,
            in_command_output: true,
            current_command: Some("echo test".to_string()),
            last_exit_code: Some(1),
            cwd: Some("/tmp".to_string()),
        };

        let shell_int2 = shell_int1.clone();

        assert_eq!(shell_int1.in_prompt, shell_int2.in_prompt);
        assert_eq!(shell_int1.current_command, shell_int2.current_command);
        assert_eq!(shell_int1.last_exit_code, shell_int2.last_exit_code);
        assert_eq!(shell_int1.cwd, shell_int2.cwd);
    }

    #[test]
    fn test_pygraphic_get_pixel_valid() {
        // Create a 2x2 pixel graphic with RGBA data
        let pixels = vec![
            255, 0, 0, 255, // Red pixel at (0, 0)
            0, 255, 0, 255, // Green pixel at (1, 0)
            0, 0, 255, 255, // Blue pixel at (0, 1)
            255, 255, 0, 255, // Yellow pixel at (1, 1)
        ];

        let graphic = PyGraphic {
            position: (0, 0),
            width: 2,
            height: 2,
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 255))); // Red
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 255))); // Green
        assert_eq!(graphic.get_pixel(0, 1), Some((0, 0, 255, 255))); // Blue
        assert_eq!(graphic.get_pixel(1, 1), Some((255, 255, 0, 255))); // Yellow
    }

    #[test]
    fn test_pygraphic_get_pixel_out_of_bounds() {
        let graphic = PyGraphic {
            position: (0, 0),
            width: 2,
            height: 2,
            pixels: vec![0; 16], // 2x2 RGBA
        };

        assert_eq!(graphic.get_pixel(2, 0), None); // X out of bounds
        assert_eq!(graphic.get_pixel(0, 2), None); // Y out of bounds
        assert_eq!(graphic.get_pixel(2, 2), None); // Both out of bounds
    }

    #[test]
    fn test_pygraphic_get_pixel_edge_cases() {
        let graphic = PyGraphic {
            position: (5, 10),
            width: 3,
            height: 3,
            pixels: vec![128; 36], // 3x3 RGBA with all values at 128
        };

        // Test valid edge pixels
        assert_eq!(graphic.get_pixel(0, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(0, 2), Some((128, 128, 128, 128)));
        assert_eq!(graphic.get_pixel(2, 2), Some((128, 128, 128, 128)));

        // Test just outside bounds
        assert_eq!(graphic.get_pixel(3, 0), None);
        assert_eq!(graphic.get_pixel(0, 3), None);
    }

    #[test]
    fn test_pygraphic_pixels_returns_copy() {
        let original_pixels = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let graphic = PyGraphic {
            position: (0, 0),
            width: 2,
            height: 1,
            pixels: original_pixels.clone(),
        };

        let retrieved = graphic.pixels();
        assert_eq!(retrieved, original_pixels);
        assert_eq!(retrieved.len(), 8); // 2 pixels * 4 channels
    }

    #[test]
    fn test_pygraphic_repr() {
        let graphic = PyGraphic {
            position: (10, 20),
            width: 100,
            height: 50,
            pixels: vec![],
        };

        let repr = graphic.__repr__().unwrap();
        assert!(repr.contains("position=(10,20)"));
        assert!(repr.contains("size=100x50"));
    }

    #[test]
    fn test_pygraphic_clone() {
        let graphic1 = PyGraphic {
            position: (5, 10),
            width: 20,
            height: 30,
            pixels: vec![1, 2, 3, 4],
        };

        let graphic2 = graphic1.clone();

        assert_eq!(graphic1.position, graphic2.position);
        assert_eq!(graphic1.width, graphic2.width);
        assert_eq!(graphic1.height, graphic2.height);
        assert_eq!(graphic1.pixels(), graphic2.pixels());
    }

    #[test]
    fn test_pygraphic_pixel_index_calculation() {
        // Test that pixel indexing is calculated correctly
        let mut pixels = vec![0u8; 16]; // 2x2 grid, RGBA

        // Manually set pixel at (1, 1) to red
        let x = 1usize;
        let y = 1usize;
        let width = 2usize;
        let idx = (y * width + x) * 4;

        pixels[idx] = 255; // R
        pixels[idx + 1] = 0; // G
        pixels[idx + 2] = 0; // B
        pixels[idx + 3] = 255; // A

        let graphic = PyGraphic {
            position: (0, 0),
            width: 2,
            height: 2,
            pixels,
        };

        assert_eq!(graphic.get_pixel(1, 1), Some((255, 0, 0, 255)));
    }

    #[test]
    fn test_line_cell_data_type_alias() {
        // Test that the LineCellData type alias works correctly
        let cell_data: LineCellData = vec![
            ('A', (255, 0, 0), (0, 0, 0), PyAttributes::default()),
            ('B', (0, 255, 0), (0, 0, 0), PyAttributes::default()),
        ];

        assert_eq!(cell_data.len(), 2);
        assert_eq!(cell_data[0].0, 'A');
        assert_eq!(cell_data[0].1, (255, 0, 0)); // Red
        assert_eq!(cell_data[1].0, 'B');
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
                ('\x00', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // NULL
                ('\x1F', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // Unit separator
                ('\x20', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // Space (32)
                ('\x21', (255, 255, 255), (0, 0, 0), PyAttributes::default()), // '!' (33)
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
        assert_eq!(line[0].0, ' '); // NULL -> space
        assert_eq!(line[1].0, ' '); // Unit separator -> space

        // Space and above should be unchanged
        assert_eq!(line[2].0, ' '); // Space unchanged
        assert_eq!(line[3].0, '!'); // '!' unchanged
    }

    #[test]
    fn test_pygraphic_alpha_channel() {
        // Test graphics with various alpha values
        let pixels = vec![
            255, 0, 0, 0, // Red, fully transparent
            0, 255, 0, 128, // Green, semi-transparent
            0, 0, 255, 255, // Blue, fully opaque
            128, 128, 128, 64, // Gray, mostly transparent
        ];

        let graphic = PyGraphic {
            position: (0, 0),
            width: 4,
            height: 1,
            pixels,
        };

        assert_eq!(graphic.get_pixel(0, 0), Some((255, 0, 0, 0)));
        assert_eq!(graphic.get_pixel(1, 0), Some((0, 255, 0, 128)));
        assert_eq!(graphic.get_pixel(2, 0), Some((0, 0, 255, 255)));
        assert_eq!(graphic.get_pixel(3, 0), Some((128, 128, 128, 64)));
    }
}
