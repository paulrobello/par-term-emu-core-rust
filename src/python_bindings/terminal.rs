//! Python bindings for the Terminal emulator
//!
//! This module contains the `PyTerminal` struct and its implementation,
//! providing the main Python interface for terminal emulation functionality.

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::color::Color;

use super::enums::PyCursorStyle;
use super::types::{LineCellData, PyAttributes, PyGraphic, PyScreenSnapshot, PyShellIntegration};

/// Python wrapper for the Terminal
#[pyclass(name = "Terminal")]
pub struct PyTerminal {
    inner: crate::terminal::Terminal,
}

#[pymethods]
impl PyTerminal {
    /// Create a new terminal with the specified dimensions
    ///
    /// Args:
    ///     cols: Number of columns (width)
    ///     rows: Number of rows (height)
    ///     scrollback: Maximum number of scrollback lines (default: 10000)
    #[new]
    #[pyo3(signature = (cols, rows, scrollback=10000))]
    fn new(cols: usize, rows: usize, scrollback: usize) -> PyResult<Self> {
        if cols == 0 || rows == 0 {
            return Err(PyValueError::new_err("Dimensions must be greater than 0"));
        }
        Ok(Self {
            inner: crate::terminal::Terminal::with_scrollback(cols, rows, scrollback),
        })
    }

    /// Process input bytes (can contain ANSI escape sequences)
    ///
    /// Args:
    ///     data: Bytes or string to process
    fn process(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.process(data);
        Ok(())
    }

    /// Process a string (convenience method)
    ///
    /// Args:
    ///     text: String to process
    fn process_str(&mut self, text: &str) -> PyResult<()> {
        self.inner.process(text.as_bytes());
        Ok(())
    }

    /// Get the terminal content as a string
    ///
    /// Returns:
    ///     String representation of the terminal buffer
    fn content(&self) -> PyResult<String> {
        Ok(self.inner.content())
    }

    /// Export entire buffer (scrollback + current screen) as plain text
    ///
    /// This exports all buffer contents with:
    /// - No styling, colors, or graphics (Sixel, etc.)
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Empty lines preserved
    ///
    /// Returns:
    ///     String containing all buffer text from scrollback through current screen
    fn export_text(&self) -> PyResult<String> {
        Ok(self.inner.export_text())
    }

    /// Export entire buffer (scrollback + current screen) with ANSI styling
    ///
    /// This exports all buffer contents with:
    /// - Full ANSI escape sequences for colors and text attributes
    /// - Trailing spaces trimmed from each line
    /// - Wrapped lines properly handled (no newline between wrapped segments)
    /// - Efficient escape sequence generation (only emits changes)
    ///
    /// Returns:
    ///     String containing all buffer text with ANSI styling
    fn export_styled(&self) -> PyResult<String> {
        Ok(self.inner.export_styled())
    }

    /// Take a screenshot of the current visible buffer
    ///
    /// Args:
    ///     format: Image format ("png", "jpeg", "svg", "bmp"). Default: "png"
    ///     font_path: Path to TTF/OTF font file. Default: None (use system font)
    ///     font_size: Font size in pixels. Default: 14.0
    ///     include_scrollback: Include scrollback buffer. Default: False
    ///     padding: Padding around content in pixels. Default: 10
    ///     quality: JPEG quality (1-100). Default: 90
    ///     render_cursor: Render cursor in screenshot. Default: False
    ///     cursor_color: RGB tuple for cursor color. Default: None (white)
    ///     sixel_mode: Sixel rendering mode ('disabled', 'pixels', 'halfblocks'). Default: 'halfblocks'
    ///     scrollback_offset: Number of lines to scroll back from current position. Default: 0
    ///     link_color: RGB tuple for link color. Default: None (use theme color)
    ///     bold_color: RGB tuple for bold text. Default: None (use theme color)
    ///     use_bold_color: Use custom bold color. Default: None (use theme setting)
    ///     minimum_contrast: Minimum contrast adjustment (0.0-1.0, iTerm2-compatible). Default: 0.0 (disabled)
    ///
    /// Returns:
    ///     Bytes of the image in the specified format
    #[pyo3(signature = (
        format = "png",
        font_path = None,
        font_size = 14.0,
        include_scrollback = false,
        padding = 10,
        quality = 90,
        render_cursor = false,
        cursor_color = None,
        sixel_mode = "halfblocks",
        scrollback_offset = 0,
        link_color = None,
        bold_color = None,
        use_bold_color = None,
        minimum_contrast = 0.0
    ))]
    #[allow(clippy::too_many_arguments)]
    fn screenshot(
        &self,
        format: &str,
        font_path: Option<String>,
        font_size: f32,
        include_scrollback: bool,
        padding: u32,
        quality: u8,
        render_cursor: bool,
        cursor_color: Option<(u8, u8, u8)>,
        sixel_mode: &str,
        scrollback_offset: usize,
        link_color: Option<(u8, u8, u8)>,
        bold_color: Option<(u8, u8, u8)>,
        use_bold_color: Option<bool>,
        minimum_contrast: f64,
    ) -> PyResult<Vec<u8>> {
        use crate::screenshot::{ImageFormat, ScreenshotConfig};

        let img_format = match format.to_lowercase().as_str() {
            "png" => ImageFormat::Png,
            "jpeg" | "jpg" => ImageFormat::Jpeg,
            "svg" => ImageFormat::Svg,
            "bmp" => ImageFormat::Bmp,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "Invalid format: {}. Use png, jpeg, svg, or bmp",
                    format
                )))
            }
        };

        let config = ScreenshotConfig {
            format: img_format,
            font_path: font_path.map(std::path::PathBuf::from),
            font_size,
            include_scrollback,
            padding_px: padding,
            quality: quality.min(100),
            render_cursor,
            cursor_color: cursor_color.unwrap_or((255, 255, 255)),
            sixel_render_mode: super::conversions::parse_sixel_mode(sixel_mode)?,
            link_color,
            bold_color,
            use_bold_color: use_bold_color.unwrap_or(false),
            minimum_contrast: minimum_contrast.clamp(0.0, 1.0),
            ..Default::default()
        };

        self.inner
            .screenshot(config, scrollback_offset)
            .map_err(|e| PyRuntimeError::new_err(format!("Screenshot error: {}", e)))
    }

    /// Take a screenshot and save to file
    ///
    /// The image format is auto-detected from the file extension if not specified.
    ///
    /// Args:
    ///     path: Output file path
    ///     format: Image format (optional, auto-detected from extension)
    ///     font_path: Path to TTF/OTF font file (optional)
    ///     font_size: Font size in pixels. Default: 14.0
    ///     include_scrollback: Include scrollback buffer. Default: False
    ///     padding: Padding around content in pixels. Default: 10
    ///     quality: JPEG quality (1-100). Default: 90
    ///     render_cursor: Render cursor in screenshot. Default: False
    ///     cursor_color: RGB tuple for cursor color. Default: None (white)
    ///     sixel_mode: Sixel rendering mode ('disabled', 'pixels', 'halfblocks'). Default: 'halfblocks'
    ///     scrollback_offset: Number of lines to scroll back from current position. Default: 0
    ///     link_color: RGB tuple for link color. Default: None (use theme color)
    ///     bold_color: RGB tuple for bold text. Default: None (use theme color)
    ///     use_bold_color: Use custom bold color. Default: None (use theme setting)
    ///     minimum_contrast: Minimum contrast adjustment (0.0-1.0, iTerm2-compatible). Default: 0.0 (disabled)
    ///
    /// Returns:
    ///     None
    #[pyo3(signature = (
        path,
        format = None,
        font_path = None,
        font_size = 14.0,
        include_scrollback = false,
        padding = 10,
        quality = 90,
        render_cursor = false,
        cursor_color = None,
        sixel_mode = "halfblocks",
        scrollback_offset = 0,
        link_color = None,
        bold_color = None,
        use_bold_color = None,
        minimum_contrast = 0.0
    ))]
    #[allow(clippy::too_many_arguments)]
    fn screenshot_to_file(
        &self,
        path: &str,
        format: Option<&str>,
        font_path: Option<String>,
        font_size: f32,
        include_scrollback: bool,
        padding: u32,
        quality: u8,
        render_cursor: bool,
        cursor_color: Option<(u8, u8, u8)>,
        sixel_mode: &str,
        scrollback_offset: usize,
        link_color: Option<(u8, u8, u8)>,
        bold_color: Option<(u8, u8, u8)>,
        use_bold_color: Option<bool>,
        minimum_contrast: f64,
    ) -> PyResult<()> {
        use std::path::Path;

        // Auto-detect format from file extension if not provided
        let detected_format = format
            .or_else(|| Path::new(path).extension().and_then(|s| s.to_str()))
            .unwrap_or("png");

        let bytes = self.screenshot(
            detected_format,
            font_path,
            font_size,
            include_scrollback,
            padding,
            quality,
            render_cursor,
            cursor_color,
            sixel_mode,
            scrollback_offset,
            link_color,
            bold_color,
            use_bold_color,
            minimum_contrast,
        )?;

        std::fs::write(path, bytes)
            .map_err(|e| PyIOError::new_err(format!("Failed to write file: {}", e)))
    }

    /// Get the current terminal dimensions
    ///
    /// Returns:
    ///     Tuple of (cols, rows)
    fn size(&self) -> PyResult<(usize, usize)> {
        Ok(self.inner.size())
    }

    /// Resize the terminal
    ///
    /// Args:
    ///     cols: New number of columns
    ///     rows: New number of rows
    fn resize(&mut self, cols: usize, rows: usize) -> PyResult<()> {
        if cols == 0 || rows == 0 {
            return Err(PyValueError::new_err("Dimensions must be greater than 0"));
        }
        self.inner.resize(cols, rows);
        Ok(())
    }

    /// Resize and set pixel dimensions for XTWINOPS reporting
    ///
    /// Args:
    ///     cols: New columns
    ///     rows: New rows
    ///     pixel_width: Text area width in pixels
    ///     pixel_height: Text area height in pixels
    #[pyo3(signature = (cols, rows, pixel_width, pixel_height))]
    fn resize_pixels(
        &mut self,
        cols: usize,
        rows: usize,
        pixel_width: usize,
        pixel_height: usize,
    ) -> PyResult<()> {
        if cols == 0 || rows == 0 {
            return Err(PyValueError::new_err("Dimensions must be greater than 0"));
        }
        self.inner.resize(cols, rows);
        self.inner.set_pixel_size(pixel_width, pixel_height);
        Ok(())
    }

    /// Reset the terminal to default state
    fn reset(&mut self) -> PyResult<()> {
        self.inner.reset();
        Ok(())
    }

    /// Get the terminal title
    ///
    /// Returns:
    ///     Current terminal title string
    fn title(&self) -> PyResult<String> {
        Ok(self.inner.title().to_string())
    }

    /// Get the cursor position
    ///
    /// Returns:
    ///     Tuple of (col, row)
    fn cursor_position(&self) -> PyResult<(usize, usize)> {
        let cursor = self.inner.cursor();
        Ok((cursor.col, cursor.row))
    }

    /// Check if cursor is visible
    ///
    /// Returns:
    ///     True if cursor is visible
    fn cursor_visible(&self) -> PyResult<bool> {
        Ok(self.inner.cursor().visible)
    }

    /// Get current Kitty Keyboard Protocol flags
    ///
    /// Returns:
    ///     Current keyboard protocol flags (u16)
    ///     Flags: 1=disambiguate, 2=report events, 4=alternate keys, 8=report all, 16=associated text
    fn keyboard_flags(&self) -> PyResult<u16> {
        Ok(self.inner.keyboard_flags())
    }

    /// Set Kitty Keyboard Protocol flags
    ///
    /// Args:
    ///     flags: Flags to set (1=disambiguate, 2=report events, 4=alternate keys, 8=report all, 16=associated text)
    ///     mode: 0=disable all, 1=set flags, 2=lock flags (default: 1)
    ///
    /// Sends: CSI = flags ; mode u
    #[pyo3(signature = (flags, mode=1))]
    fn set_keyboard_flags(&mut self, flags: u16, mode: u8) -> PyResult<()> {
        let sequence = format!("\x1b[={};{}u", flags, mode);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Get current terminal conformance level
    ///
    /// Returns:
    ///     Conformance level as integer (1=VT100, 2=VT220, 3=VT320, 4=VT420, 5=VT520)
    fn conformance_level(&self) -> PyResult<u8> {
        Ok(self.inner.conformance_level().level())
    }

    /// Get conformance level name
    ///
    /// Returns:
    ///     String name of conformance level ("VT100", "VT220", "VT320", "VT420", "VT520")
    fn conformance_level_name(&self) -> PyResult<String> {
        Ok(self.inner.conformance_level().to_string())
    }

    /// Set terminal conformance level
    ///
    /// Args:
    ///     level: Conformance level (1 or 61=VT100, 2 or 62=VT220, 3 or 63=VT320, 4 or 64=VT420, 5 or 65=VT520)
    ///     c1_mode: 8-bit control mode (0=7-bit, 1 or 2=8-bit, default: 2)
    ///
    /// Sends: CSI level ; c1_mode " p
    #[pyo3(signature = (level, c1_mode=2))]
    fn set_conformance_level(&mut self, level: u16, c1_mode: u8) -> PyResult<()> {
        // Validate level parameter
        let valid_levels = [1, 2, 3, 4, 5, 61, 62, 63, 64, 65];
        if !valid_levels.contains(&level) {
            return Err(PyValueError::new_err(format!(
                "Invalid conformance level: {}. Valid values: 1-5 or 61-65",
                level
            )));
        }

        let sequence = format!("\x1b[{};{}\"p", level, c1_mode);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Get warning bell volume
    ///
    /// Returns:
    ///     Volume level (0=off, 1-8=volume levels)
    fn warning_bell_volume(&self) -> PyResult<u8> {
        Ok(self.inner.warning_bell_volume())
    }

    /// Set warning bell volume (VT520)
    ///
    /// Args:
    ///     volume: Volume level (0=off, 1=low, 2-4=medium levels, 5-8=high levels)
    ///
    /// Sends: CSI volume SP t
    fn set_warning_bell_volume(&mut self, volume: u8) -> PyResult<()> {
        if volume > 8 {
            return Err(PyValueError::new_err(format!(
                "Invalid volume: {}. Valid range: 0-8",
                volume
            )));
        }

        let sequence = format!("\x1b[{} t", volume);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Get margin bell volume
    ///
    /// Returns:
    ///     Volume level (0=off, 1-8=volume levels)
    fn margin_bell_volume(&self) -> PyResult<u8> {
        Ok(self.inner.margin_bell_volume())
    }

    /// Set margin bell volume (VT520)
    ///
    /// Args:
    ///     volume: Volume level (0=off, 1=low, 2-4=medium levels, 5-8=high levels)
    ///
    /// Sends: CSI volume SP u
    fn set_margin_bell_volume(&mut self, volume: u8) -> PyResult<()> {
        if volume > 8 {
            return Err(PyValueError::new_err(format!(
                "Invalid volume: {}. Valid range: 0-8",
                volume
            )));
        }

        let sequence = format!("\x1b[{} u", volume);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Query Kitty Keyboard Protocol flags (sends CSI ? u)
    ///
    /// Returns:
    ///     Query sequence sent to terminal (response will be in drain_responses())
    fn query_keyboard_flags(&mut self) -> PyResult<()> {
        self.inner.process(b"\x1b[?u");
        Ok(())
    }

    /// Get insert mode (IRM - Mode 4) state
    ///
    /// Returns:
    ///     True if insert mode is enabled (characters are inserted), False if replace mode (default)
    fn insert_mode(&self) -> PyResult<bool> {
        Ok(self.inner.insert_mode())
    }

    /// Get line feed/new line mode (LNM - Mode 20) state
    ///
    /// Returns:
    ///     True if LNM is enabled (LF does CR+LF), False if LF only (default)
    fn line_feed_new_line_mode(&self) -> PyResult<bool> {
        Ok(self.inner.line_feed_new_line_mode())
    }

    /// Push current keyboard flags to stack and set new flags
    ///
    /// Args:
    ///     flags: New flags to set
    ///
    /// Sends: CSI > flags u
    fn push_keyboard_flags(&mut self, flags: u16) -> PyResult<()> {
        let sequence = format!("\x1b[>{}u", flags);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Pop keyboard flags from stack
    ///
    /// Args:
    ///     count: Number of flags to pop from stack (default: 1)
    ///
    /// Sends: CSI < count u
    #[pyo3(signature = (count=1))]
    fn pop_keyboard_flags(&mut self, count: usize) -> PyResult<()> {
        let sequence = format!("\x1b[<{}u", count);
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Get clipboard content (OSC 52)
    ///
    /// Returns:
    ///     Clipboard content as string, or None if empty
    fn clipboard(&self) -> PyResult<Option<String>> {
        Ok(self.inner.clipboard().map(|s| s.to_string()))
    }

    /// Set clipboard content programmatically
    ///
    /// This bypasses OSC 52 sequences and directly sets the clipboard.
    /// Useful for integration with system clipboard or testing.
    ///
    /// Args:
    ///     content: Content to set (None to clear)
    fn set_clipboard(&mut self, content: Option<String>) -> PyResult<()> {
        self.inner.set_clipboard(content);
        Ok(())
    }

    /// Check if clipboard read operations are allowed
    ///
    /// Returns:
    ///     True if OSC 52 queries (ESC ] 52 ; c ; ? ST) are allowed
    fn allow_clipboard_read(&self) -> PyResult<bool> {
        Ok(self.inner.allow_clipboard_read())
    }

    /// Set whether clipboard read operations are allowed
    ///
    /// When disabled (default), OSC 52 queries are silently ignored for security.
    /// When enabled, terminal applications can query clipboard contents.
    ///
    /// Args:
    ///     allow: True to allow clipboard read, False to block (default)
    fn set_allow_clipboard_read(&mut self, allow: bool) -> PyResult<()> {
        self.inner.set_allow_clipboard_read(allow);
        Ok(())
    }

    /// Get default foreground color (OSC 10)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn default_fg(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.default_fg().to_rgb())
    }

    /// Set default foreground color (OSC 10)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_default_fg(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_default_fg(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Query default foreground color (OSC 10)
    ///
    /// Sends OSC 10 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 10 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_fg(&mut self) -> PyResult<()> {
        self.inner.process(b"\x1b]10;?\x1b\\");
        Ok(())
    }

    /// Get default background color (OSC 11)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn default_bg(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.default_bg().to_rgb())
    }

    /// Set default background color (OSC 11)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_default_bg(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_default_bg(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Query default background color (OSC 11)
    ///
    /// Sends OSC 11 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 11 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_bg(&mut self) -> PyResult<()> {
        self.inner.process(b"\x1b]11;?\x1b\\");
        Ok(())
    }

    /// Get cursor color (OSC 12)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn cursor_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.cursor_color().to_rgb())
    }

    /// Set cursor color (OSC 12)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_cursor_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_cursor_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Query cursor color (OSC 12)
    ///
    /// Sends OSC 12 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 12 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_cursor_color(&mut self) -> PyResult<()> {
        self.inner.process(b"\x1b]12;?\x1b\\");
        Ok(())
    }

    /// Set ANSI palette color (0-15)
    ///
    /// Args:
    ///     index: Palette index (0-15)
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    ///
    /// Raises:
    ///     ValueError: If index is not in range 0-15
    fn set_ansi_palette_color(&mut self, index: usize, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner
            .set_ansi_palette_color(index, Color::Rgb(r, g, b))
            .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
        Ok(())
    }

    /// Set link/hyperlink color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_link_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_link_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set bold text color (when use_bold_color is enabled)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_bold_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_bold_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set cursor guide color (vertical line following cursor)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_cursor_guide_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_cursor_guide_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set badge color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_badge_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_badge_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set match/search highlight color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_match_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_match_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set selection background color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_selection_bg_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_selection_bg_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Set selection foreground/text color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_selection_fg_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        self.inner.set_selection_fg_color(Color::Rgb(r, g, b));
        Ok(())
    }

    /// Enable/disable custom bold color
    ///
    /// When enabled, bold text uses set_bold_color() instead of bright ANSI variant.
    ///
    /// Args:
    ///     use_bold: Whether to use custom bold color
    fn set_use_bold_color(&mut self, use_bold: bool) -> PyResult<()> {
        self.inner.set_use_bold_color(use_bold);
        Ok(())
    }

    /// Enable/disable custom underline color
    ///
    /// When enabled, underlined text uses a custom underline color.
    ///
    /// Args:
    ///     use_underline: Whether to use custom underline color
    fn set_use_underline_color(&mut self, use_underline: bool) -> PyResult<()> {
        self.inner.set_use_underline_color(use_underline);
        Ok(())
    }

    /// Get cursor style (DECSCUSR)
    ///
    /// Returns:
    ///     CursorStyle enum value
    fn cursor_style(&self) -> PyResult<PyCursorStyle> {
        Ok(self.inner.cursor().style().into())
    }

    /// Set cursor style (DECSCUSR)
    ///
    /// This is equivalent to sending CSI <n> SP q escape sequence.
    ///
    /// Args:
    ///     style: CursorStyle enum value (e.g., CursorStyle.BlinkingBlock)
    fn set_cursor_style(&mut self, style: PyCursorStyle) -> PyResult<()> {
        // Send DECSCUSR escape sequence (CSI <n> SP q)
        let sequence = format!(
            "\x1b[{} q",
            match style {
                PyCursorStyle::BlinkingBlock => 1,
                PyCursorStyle::SteadyBlock => 2,
                PyCursorStyle::BlinkingUnderline => 3,
                PyCursorStyle::SteadyUnderline => 4,
                PyCursorStyle::BlinkingBar => 5,
                PyCursorStyle::SteadyBar => 6,
            }
        );
        self.inner.process(sequence.as_bytes());
        Ok(())
    }

    /// Get scrollback content as a list of strings
    ///
    /// Returns:
    ///     List of scrollback lines
    fn scrollback(&self) -> PyResult<Vec<String>> {
        Ok(self.inner.scrollback())
    }

    /// Get the number of scrollback lines
    ///
    /// Returns:
    ///     Number of lines in scrollback buffer
    fn scrollback_len(&self) -> PyResult<usize> {
        Ok(self.inner.grid().scrollback_len())
    }

    /// Get a specific line from the scrollback buffer with full cell data
    ///
    /// Args:
    ///     index: Scrollback line index (0 = oldest, scrollback_len()-1 = most recent)
    ///
    /// Returns:
    ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes),
    ///     or None if index is out of bounds
    #[allow(clippy::type_complexity)]
    fn scrollback_line(
        &self,
        index: usize,
    ) -> PyResult<Option<Vec<(char, (u8, u8, u8), (u8, u8, u8), PyAttributes)>>> {
        let grid = self.inner.grid();
        if let Some(line) = grid.scrollback_line(index) {
            let cells: Vec<_> = line
                .iter()
                .map(|cell| {
                    (
                        cell.c,
                        cell.fg.to_rgb(),
                        cell.bg.to_rgb(),
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
                            hyperlink_id: cell.flags.hyperlink_id,
                        },
                    )
                })
                .collect();
            Ok(Some(cells))
        } else {
            Ok(None)
        }
    }

    /// Get a specific line from the terminal buffer
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     String content of the specified row, or None if row is out of bounds
    fn get_line(&self, row: usize) -> PyResult<Option<String>> {
        if let Some(line) = self.inner.grid().row(row) {
            Ok(Some(line.iter().map(|cell| cell.c).collect()))
        } else {
            Ok(None)
        }
    }

    /// Get a cell's character at the specified position
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Character at the position, or None if out of bounds
    fn get_char(&self, col: usize, row: usize) -> PyResult<Option<char>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(Some(cell.c))
        } else {
            Ok(None)
        }
    }

    /// Check if a line is wrapped (continues to the next line)
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     True if the line wraps to the next row, False otherwise
    fn is_line_wrapped(&self, row: usize) -> PyResult<bool> {
        Ok(self.inner.active_grid().is_line_wrapped(row))
    }

    /// Get a cell's foreground color at the specified position
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b) values, or None if out of bounds
    fn get_fg_color(&self, col: usize, row: usize) -> PyResult<Option<(u8, u8, u8)>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(Some(cell.fg.to_rgb()))
        } else {
            Ok(None)
        }
    }

    /// Get a cell's background color at the specified position
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b) values, or None if out of bounds
    fn get_bg_color(&self, col: usize, row: usize) -> PyResult<Option<(u8, u8, u8)>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(Some(cell.bg.to_rgb()))
        } else {
            Ok(None)
        }
    }

    /// Get a cell's underline color at the specified position (SGR 58)
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Tuple of (r, g, b) values, or None if no underline color set or out of bounds
    fn get_underline_color(&self, col: usize, row: usize) -> PyResult<Option<(u8, u8, u8)>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(cell.underline_color.map(|c| c.to_rgb()))
        } else {
            Ok(None)
        }
    }

    /// Get cell attributes at the specified position
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Dictionary with boolean flags: bold, italic, underline, etc., or None if out of bounds
    fn get_attributes(&self, col: usize, row: usize) -> PyResult<Option<PyAttributes>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(Some(PyAttributes {
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
                hyperlink_id: cell.flags.hyperlink_id,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get hyperlink URL at the specified position
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     URL string if the cell has a hyperlink, or None if no hyperlink or out of bounds
    fn get_hyperlink(&self, col: usize, row: usize) -> PyResult<Option<String>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            if let Some(id) = cell.flags.hyperlink_id {
                return Ok(self.inner.get_hyperlink_url(id));
            }
        }
        Ok(None)
    }

    /// Get all cell data for a row in a single atomic operation
    ///
    /// This method retrieves all cell information for an entire row atomically,
    /// preventing race conditions in multi-threaded scenarios.
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes) for each column,
    ///     or empty list if row is out of bounds
    fn get_line_cells(&self, row: usize) -> PyResult<LineCellData> {
        let grid = self.inner.active_grid();
        let rows = grid.rows();

        if row >= rows {
            return Ok(Vec::new());
        }

        let cols = grid.cols();
        let result = (0..cols)
            .filter_map(|col| {
                grid.get(col, row).map(|cell| {
                    (
                        cell.c,
                        cell.fg.to_rgb(),
                        cell.bg.to_rgb(),
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
                            hyperlink_id: cell.flags.hyperlink_id,
                        },
                    )
                })
            })
            .collect();

        Ok(result)
    }

    /// Create atomic snapshot of current screen state
    ///
    /// Captures all lines, cursor state, and screen identity atomically.
    /// The snapshot is immutable and will not change even if the terminal
    /// state changes (e.g., alternate screen switches).
    ///
    /// Returns:
    ///     ScreenSnapshot with all terminal state
    fn create_snapshot(&self) -> PyResult<PyScreenSnapshot> {
        // Get current grid (will be either primary or alternate)
        let grid = self.inner.active_grid();
        let rows = grid.rows();
        let cols = grid.cols();

        // Capture all lines while holding terminal reference
        let mut lines = Vec::with_capacity(rows);
        let mut wrapped_lines = Vec::with_capacity(rows);
        for row in 0..rows {
            let mut line = Vec::with_capacity(cols);
            for col in 0..cols {
                if let Some(cell) = grid.get(col, row) {
                    line.push((
                        cell.c,
                        cell.fg.to_rgb(),
                        cell.bg.to_rgb(),
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
                            hyperlink_id: cell.flags.hyperlink_id,
                        },
                    ));
                } else {
                    // Empty cell
                    line.push((' ', (0, 0, 0), (0, 0, 0), PyAttributes::default()));
                }
            }
            lines.push(line);
            wrapped_lines.push(grid.is_line_wrapped(row));
        }

        let cursor = self.inner.cursor();

        Ok(PyScreenSnapshot {
            lines,
            wrapped_lines,
            cursor_pos: (cursor.col, cursor.row),
            cursor_visible: cursor.visible,
            cursor_style: cursor.style.into(),
            is_alt_screen: self.inner.is_alt_screen_active(),
            generation: 0, // Terminal doesn't have generation tracking
            size: (cols, rows),
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        let (cols, rows) = self.inner.size();
        Ok(format!("Terminal(cols={}, rows={})", cols, rows))
    }

    fn __str__(&self) -> PyResult<String> {
        Ok(self.inner.content())
    }

    // Advanced features

    /// Check if alternate screen is active
    ///
    /// Returns:
    ///     True if alternate screen is active
    fn is_alt_screen_active(&self) -> PyResult<bool> {
        Ok(self.inner.is_alt_screen_active())
    }

    /// Get mouse tracking mode
    ///
    /// Returns:
    ///     String representing the mouse mode: "off", "normal", "button", "any"
    fn mouse_mode(&self) -> PyResult<String> {
        use crate::mouse::MouseMode;
        let mode = match self.inner.mouse_mode() {
            MouseMode::Off => "off",
            MouseMode::X10 => "x10",
            MouseMode::Normal => "normal",
            MouseMode::ButtonEvent => "button",
            MouseMode::AnyEvent => "any",
        };
        Ok(mode.to_string())
    }

    /// Check if focus tracking is enabled
    ///
    /// Returns:
    ///     True if focus tracking is enabled
    fn focus_tracking(&self) -> PyResult<bool> {
        Ok(self.inner.focus_tracking())
    }

    /// Check if bracketed paste mode is enabled
    ///
    /// Returns:
    ///     True if bracketed paste mode is enabled
    fn bracketed_paste(&self) -> PyResult<bool> {
        Ok(self.inner.bracketed_paste())
    }

    /// Check if synchronized updates mode is enabled (DEC 2026)
    ///
    /// Returns:
    ///     True if synchronized updates mode is enabled
    fn synchronized_updates(&self) -> PyResult<bool> {
        Ok(self.inner.synchronized_updates())
    }

    /// Manually flush the synchronized update buffer
    ///
    /// This is useful for flushing buffered updates without disabling synchronized mode.
    /// Note: The buffer is automatically flushed when synchronized mode is disabled via CSI ? 2026 l
    fn flush_synchronized_updates(&mut self) -> PyResult<()> {
        self.inner.flush_synchronized_updates();
        Ok(())
    }

    /// Simulate a mouse event and get the escape sequence
    ///
    /// Args:
    ///     button: Mouse button (0=left, 1=middle, 2=right)
    ///     col: Column position (0-based)
    ///     row: Row position (0-based)
    ///     pressed: True for press, False for release
    ///
    /// Returns:
    ///     Bytes representing the mouse event sequence
    fn simulate_mouse_event(
        &mut self,
        button: u8,
        col: usize,
        row: usize,
        pressed: bool,
    ) -> PyResult<Vec<u8>> {
        use crate::mouse::MouseEvent;
        let event = MouseEvent::new(button, col, row, pressed, 0);
        Ok(self.inner.report_mouse(event))
    }

    /// Get focus in event sequence
    ///
    /// Returns:
    ///     Bytes for focus in event (if focus tracking is enabled)
    fn get_focus_in_event(&self) -> PyResult<Vec<u8>> {
        Ok(self.inner.report_focus_in())
    }

    /// Get focus out event sequence
    ///
    /// Returns:
    ///     Bytes for focus out event (if focus tracking is enabled)
    fn get_focus_out_event(&self) -> PyResult<Vec<u8>> {
        Ok(self.inner.report_focus_out())
    }

    /// Get bracketed paste start sequence
    ///
    /// Returns:
    ///     Bytes for paste start (if bracketed paste is enabled)
    fn get_paste_start(&self) -> PyResult<Vec<u8>> {
        Ok(self.inner.bracketed_paste_start().to_vec())
    }

    /// Get bracketed paste end sequence
    ///
    /// Returns:
    ///     Bytes for paste end (if bracketed paste is enabled)
    fn get_paste_end(&self) -> PyResult<Vec<u8>> {
        Ok(self.inner.bracketed_paste_end().to_vec())
    }

    /// Paste text content into terminal with bracketed paste support
    ///
    /// If bracketed paste mode is enabled, wraps the content with ESC[200~ and ESC[201~
    /// Otherwise, processes the content directly
    ///
    /// Args:
    ///     content: String content to paste
    fn paste(&mut self, content: &str) -> PyResult<()> {
        self.inner.paste(content);
        Ok(())
    }

    /// Get shell integration state
    ///
    /// Returns:
    ///     Dictionary with shell integration info
    fn shell_integration_state(&self) -> PyResult<PyShellIntegration> {
        let si = self.inner.shell_integration();
        Ok(PyShellIntegration {
            in_prompt: si.in_prompt(),
            in_command_input: si.in_command_input(),
            in_command_output: si.in_command_output(),
            current_command: si.command().map(|s| s.to_string()),
            last_exit_code: si.exit_code(),
            cwd: si.cwd().map(|s| s.to_string()),
        })
    }

    // Sixel graphics methods

    /// Get graphics that overlap the specified row
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     List of graphics that overlap the given row
    fn graphics_at_row(&self, row: usize) -> PyResult<Vec<PyGraphic>> {
        let graphics = self.inner.graphics_at_row(row);
        Ok(graphics.iter().map(|g| PyGraphic::from(*g)).collect())
    }

    /// Get total number of graphics
    ///
    /// Returns:
    ///     Total count of Sixel graphics
    fn graphics_count(&self) -> PyResult<usize> {
        Ok(self.inner.graphics_count())
    }

    /// Clear all graphics
    fn clear_graphics(&mut self) -> PyResult<()> {
        self.inner.clear_graphics();
        Ok(())
    }

    // Device query response methods

    /// Drain and return pending device query responses
    ///
    /// Device queries like DA (Device Attributes) and DSR (Device Status Report)
    /// generate responses that are buffered. This method retrieves and clears them.
    ///
    /// Returns:
    ///     Bytes containing all pending responses
    fn drain_responses(&mut self) -> PyResult<Vec<u8>> {
        Ok(self.inner.drain_responses())
    }

    /// Check if there are pending device query responses
    ///
    /// Returns:
    ///     True if there are responses waiting to be retrieved
    fn has_pending_responses(&self) -> PyResult<bool> {
        Ok(self.inner.has_pending_responses())
    }

    // Notification methods (OSC 9 / OSC 777)

    /// Check if there are pending notifications
    ///
    /// Returns:
    ///     True if there are notifications waiting to be retrieved
    fn has_notifications(&self) -> PyResult<bool> {
        Ok(self.inner.has_notifications())
    }

    /// Get all pending notifications
    ///
    /// Returns a list of tuples: [(title, message), ...]
    /// For OSC 9 notifications, title will be empty string.
    /// Clears the notification queue after retrieval.
    ///
    /// Returns:
    ///     List of (title, message) tuples
    fn take_notifications(&mut self) -> PyResult<Vec<(String, String)>> {
        let notifications = self.inner.take_notifications();
        Ok(notifications
            .into_iter()
            .map(|n| (n.title, n.message))
            .collect())
    }

    /// Get all pending notifications (alias for take_notifications)
    ///
    /// Returns a list of tuples: [(title, message), ...]
    /// Clears the notification queue after retrieval.
    ///
    /// Returns:
    ///     List of (title, message) tuples
    fn drain_notifications(&mut self) -> PyResult<Vec<(String, String)>> {
        self.take_notifications()
    }

    /// Get a debug snapshot of the current buffer state
    ///
    /// Returns:
    ///     String containing a formatted view of the buffer
    fn debug_snapshot_buffer(&self) -> PyResult<String> {
        let grid = self.inner.active_grid();
        Ok(grid.debug_snapshot())
    }

    /// Get a debug snapshot of the grid
    ///
    /// Returns:
    ///     String containing a formatted view of the grid
    fn debug_snapshot_grid(&self) -> PyResult<String> {
        Ok(self.inner.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the primary screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the primary buffer
    fn debug_snapshot_primary(&self) -> PyResult<String> {
        Ok(self.inner.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the alternate screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the alternate buffer
    fn debug_snapshot_alt(&self) -> PyResult<String> {
        Ok(self.inner.alt_grid().debug_snapshot())
    }

    /// Log a debug snapshot with a label
    ///
    /// Args:
    ///     label: Description of this snapshot
    fn debug_log_snapshot(&self, label: &str) -> PyResult<()> {
        use crate::debug;
        let grid = self.inner.active_grid();
        let snapshot = grid.debug_snapshot();
        debug::log_buffer_snapshot(label, grid.rows(), grid.cols(), &snapshot);
        Ok(())
    }

    /// Get current working directory from shell integration (OSC 7)
    ///
    /// Returns the directory path reported by the shell via OSC 7 sequences,
    /// or None if no directory has been reported yet.
    ///
    /// Returns:
    ///     Optional string with current directory path
    fn current_directory(&self) -> PyResult<Option<String>> {
        Ok(self.inner.current_directory().map(|s| s.to_string()))
    }

    /// Check if OSC 7 directory tracking is enabled
    ///
    /// Returns:
    ///     True if OSC 7 sequences are accepted, False otherwise
    fn accept_osc7(&self) -> PyResult<bool> {
        Ok(self.inner.accept_osc7())
    }

    /// Set whether OSC 7 directory tracking sequences are accepted
    ///
    /// When disabled, OSC 7 sequences are silently ignored.
    /// When enabled (default), allows shell to report current working directory.
    ///
    /// Args:
    ///     accept: True to accept OSC 7 (default), False to ignore
    fn set_accept_osc7(&mut self, accept: bool) -> PyResult<()> {
        self.inner.set_accept_osc7(accept);
        Ok(())
    }

    /// Check if insecure sequence filtering is enabled
    ///
    /// Returns:
    ///     True if insecure sequences are blocked, False otherwise
    fn disable_insecure_sequences(&self) -> PyResult<bool> {
        Ok(self.inner.disable_insecure_sequences())
    }

    /// Set whether to filter potentially insecure escape sequences
    ///
    /// When enabled, certain sequences that could pose security risks are blocked:
    /// - OSC 52 (clipboard operations - can leak data)
    /// - OSC 8 (hyperlinks - can be used for phishing)
    /// - OSC 9/777 (notifications - can be annoying/misleading)
    /// - Sixel graphics (can consume excessive memory)
    ///
    /// When disabled (default), all standard sequences are processed normally.
    ///
    /// Args:
    ///     disable: True to block insecure sequences, False to allow (default)
    fn set_disable_insecure_sequences(&mut self, disable: bool) -> PyResult<()> {
        self.inner.set_disable_insecure_sequences(disable);
        Ok(())
    }

    /// Get current debug information as a dictionary
    ///
    /// Returns:
    ///     Dictionary containing terminal state for debugging
    fn debug_info(&self) -> PyResult<HashMap<String, String>> {
        let mut info = HashMap::new();
        let (cols, rows) = self.inner.size();
        let cursor = self.inner.cursor();

        info.insert("size".to_string(), format!("{}x{}", cols, rows));
        info.insert(
            "cursor_pos".to_string(),
            format!("({},{})", cursor.col, cursor.row),
        );
        info.insert("cursor_visible".to_string(), cursor.visible.to_string());
        info.insert(
            "alt_screen_active".to_string(),
            self.inner.is_alt_screen_active().to_string(),
        );
        info.insert(
            "scrollback_len".to_string(),
            self.inner.scrollback().len().to_string(),
        );
        info.insert("title".to_string(), self.inner.title().to_string());

        Ok(info)
    }

    // ========== Text Extraction Utilities ==========

    /// Get word at cursor position
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///     word_chars: Optional custom word characters (default: "/-+\\~_." iTerm2-compatible)
    ///
    /// Returns:
    ///     Word at position or None if not on a word
    fn get_word_at(
        &self,
        col: usize,
        row: usize,
        word_chars: Option<&str>,
    ) -> PyResult<Option<String>> {
        Ok(self.inner.get_word_at(col, row, word_chars))
    }

    /// Get URL at cursor position
    ///
    /// Detects URLs with schemes: http://, https://, ftp://, file://, mailto:, ssh://
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///
    /// Returns:
    ///     URL at position or None if not on a URL
    fn get_url_at(&self, col: usize, row: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_url_at(col, row))
    }

    /// Get full logical line following wrapping
    ///
    /// Args:
    ///     row: Row position (0-indexed)
    ///
    /// Returns:
    ///     Complete unwrapped line or None if row is invalid
    fn get_line_unwrapped(&self, row: usize) -> PyResult<Option<String>> {
        Ok(self.inner.get_line_unwrapped(row))
    }

    /// Get word boundaries at cursor position for smart selection
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///     word_chars: Optional custom word characters
    ///
    /// Returns:
    ///     ((start_col, start_row), (end_col, end_row)) or None if not on a word
    #[allow(clippy::type_complexity)]
    fn select_word(
        &self,
        col: usize,
        row: usize,
        word_chars: Option<&str>,
    ) -> PyResult<Option<((usize, usize), (usize, usize))>> {
        Ok(self.inner.select_word(col, row, word_chars))
    }

    // ========== Content Search ==========

    /// Find all occurrences of text in the visible screen
    ///
    /// Args:
    ///     pattern: Text to search for
    ///     case_sensitive: Whether search is case-sensitive (default: True)
    ///
    /// Returns:
    ///     List of (col, row) positions where pattern was found
    #[pyo3(signature = (pattern, case_sensitive = true))]
    fn find_text(&self, pattern: &str, case_sensitive: bool) -> PyResult<Vec<(usize, usize)>> {
        Ok(self.inner.find_text(pattern, case_sensitive))
    }

    /// Find next occurrence of text from given position
    ///
    /// Args:
    ///     pattern: Text to search for
    ///     from_col: Starting column position
    ///     from_row: Starting row position
    ///     case_sensitive: Whether search is case-sensitive (default: True)
    ///
    /// Returns:
    ///     (col, row) of next match, or None if not found
    #[pyo3(signature = (pattern, from_col, from_row, case_sensitive = true))]
    fn find_next(
        &self,
        pattern: &str,
        from_col: usize,
        from_row: usize,
        case_sensitive: bool,
    ) -> PyResult<Option<(usize, usize)>> {
        Ok(self
            .inner
            .find_next(pattern, from_col, from_row, case_sensitive))
    }

    // ========== Buffer Statistics ==========

    /// Get terminal statistics
    ///
    /// Returns:
    ///     Dictionary with statistics: cols, rows, scrollback_lines, total_cells,
    ///     non_whitespace_lines, graphics_count, estimated_memory_bytes
    fn get_stats(&self) -> PyResult<HashMap<String, usize>> {
        let stats = self.inner.get_stats();
        let mut result = HashMap::new();
        result.insert("cols".to_string(), stats.cols);
        result.insert("rows".to_string(), stats.rows);
        result.insert("scrollback_lines".to_string(), stats.scrollback_lines);
        result.insert("total_cells".to_string(), stats.total_cells);
        result.insert(
            "non_whitespace_lines".to_string(),
            stats.non_whitespace_lines,
        );
        result.insert("graphics_count".to_string(), stats.graphics_count);
        result.insert(
            "estimated_memory_bytes".to_string(),
            stats.estimated_memory_bytes,
        );
        result.insert("hyperlink_count".to_string(), stats.hyperlink_count);
        result.insert("hyperlink_memory_bytes".to_string(), stats.hyperlink_memory_bytes);
        result.insert("color_stack_depth".to_string(), stats.color_stack_depth);
        result.insert("title_stack_depth".to_string(), stats.title_stack_depth);
        result.insert("keyboard_stack_depth".to_string(), stats.keyboard_stack_depth);
        result.insert("response_buffer_size".to_string(), stats.response_buffer_size);
        result.insert("dirty_row_count".to_string(), stats.dirty_row_count);
        result.insert("pending_bell_events".to_string(), stats.pending_bell_events);
        result.insert("pending_terminal_events".to_string(), stats.pending_terminal_events);
        Ok(result)
    }

    /// Count non-whitespace lines in visible screen
    ///
    /// Returns:
    ///     Number of lines containing non-whitespace characters
    fn count_non_whitespace_lines(&self) -> PyResult<usize> {
        Ok(self.inner.count_non_whitespace_lines())
    }

    /// Get scrollback usage
    ///
    /// Returns:
    ///     Tuple of (used_lines, max_capacity)
    fn get_scrollback_usage(&self) -> PyResult<(usize, usize)> {
        Ok(self.inner.get_scrollback_usage())
    }

    // ========== Advanced Text Selection ==========

    /// Find matching bracket/parenthesis at cursor position
    ///
    /// Supports: (), [], {}, <>
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///
    /// Returns:
    ///     (col, row) position of matching bracket, or None
    fn find_matching_bracket(&self, col: usize, row: usize) -> PyResult<Option<(usize, usize)>> {
        Ok(self.inner.find_matching_bracket(col, row))
    }

    /// Select text within semantic delimiters
    ///
    /// Extracts content between matching delimiters around cursor.
    /// Supports: (), [], {}, <>, "", '', ``
    ///
    /// Args:
    ///     col: Column position (0-indexed)
    ///     row: Row position (0-indexed)
    ///     delimiters: String of delimiters to check (e.g., "()[]{}\"'")
    ///
    /// Returns:
    ///     Content between delimiters, or None if not inside delimiters
    ///
    /// Example:
    ///     # Cursor inside "hello world"
    ///     text = term.select_semantic_region(10, 0, "\"")  # Returns "hello world"
    fn select_semantic_region(
        &self,
        col: usize,
        row: usize,
        delimiters: &str,
    ) -> PyResult<Option<String>> {
        Ok(self.inner.select_semantic_region(col, row, delimiters))
    }

    /// Export terminal content as HTML
    ///
    /// Args:
    ///     include_styles: Whether to include full HTML document with CSS (default: True)
    ///
    /// Returns:
    ///     HTML string with terminal content and styling
    ///
    /// When include_styles is True, returns a complete HTML document.
    /// When False, returns just the styled content (useful for embedding).
    #[pyo3(signature = (include_styles = true))]
    fn export_html(&self, include_styles: bool) -> PyResult<String> {
        Ok(self.inner.export_html(include_styles))
    }

    // ========== Static Utility Methods ==========

    /// Strip ANSI escape sequences from text
    ///
    /// Args:
    ///     text: Text containing ANSI codes
    ///
    /// Returns:
    ///     Text with all ANSI sequences removed
    #[staticmethod]
    fn strip_ansi(text: &str) -> PyResult<String> {
        Ok(crate::ansi_utils::strip_ansi(text))
    }

    /// Measure text width without ANSI codes
    ///
    /// Accounts for wide characters (CJK, emoji) and strips ANSI sequences.
    ///
    /// Args:
    ///     text: Text to measure
    ///
    /// Returns:
    ///     Display width in columns
    #[staticmethod]
    fn measure_text_width(text: &str) -> PyResult<usize> {
        Ok(crate::ansi_utils::measure_text_width(text))
    }

    /// Parse color from string (hex, rgb, or name)
    ///
    /// Supported formats:
    /// - Hex: "#RRGGBB" or "#RGB"
    /// - RGB: "rgb(r, g, b)"
    /// - Names: "red", "blue", "green", etc.
    ///
    /// Args:
    ///     color_string: Color specification
    ///
    /// Returns:
    ///     RGB tuple (r, g, b) or None if invalid
    #[staticmethod]
    fn parse_color(color_string: &str) -> PyResult<Option<(u8, u8, u8)>> {
        if let Some(color) = crate::ansi_utils::parse_color(color_string) {
            Ok(Some(color.to_rgb()))
        } else {
            Ok(None)
        }
    }

    /// Get Sixel resource limits (max width, height, repeat)
    ///
    /// Returns:
    ///     Tuple of (max_width_px, max_height_px, max_repeat)
    fn get_sixel_limits(&self) -> PyResult<(usize, usize, usize)> {
        let limits = self.inner.sixel_limits();
        Ok((limits.max_width, limits.max_height, limits.max_repeat))
    }

    /// Set Sixel resource limits (max width, height, repeat)
    ///
    /// Args:
    ///     max_width: Maximum Sixel bitmap width in pixels
    ///     max_height: Maximum Sixel bitmap height in pixels
    ///     max_repeat: Maximum repeat count for !Pn sequences
    ///
    /// Limits are clamped to safe hard maxima at the Rust layer.
    fn set_sixel_limits(
        &mut self,
        max_width: usize,
        max_height: usize,
        max_repeat: usize,
    ) -> PyResult<()> {
        self.inner
            .set_sixel_limits(max_width, max_height, max_repeat);
        Ok(())
    }

    /// Get maximum number of Sixel graphics retained
    ///
    /// Returns:
    ///     Maximum number of in-memory Sixel graphics for this terminal
    fn get_sixel_graphics_limit(&self) -> PyResult<usize> {
        Ok(self.inner.max_sixel_graphics())
    }

    /// Set maximum number of Sixel graphics retained
    ///
    /// Args:
    ///     max_graphics: Maximum number of in-memory Sixel graphics
    ///
    /// Oldest graphics are dropped if the new limit is lower than the
    /// current number of graphics. The value is clamped to a safe range.
    fn set_sixel_graphics_limit(&mut self, max_graphics: usize) -> PyResult<()> {
        self.inner.set_max_sixel_graphics(max_graphics);
        Ok(())
    }

    /// Get count of Sixel graphics dropped due to limits
    ///
    /// Returns:
    ///     Number of Sixel graphics that have been dropped because of size or count limits
    fn get_dropped_sixel_graphics(&self) -> PyResult<usize> {
        Ok(self.inner.dropped_sixel_graphics())
    }

    /// Get Sixel statistics as a dictionary
    ///
    /// Returns:
    ///     {
    ///       "max_width_px": int,
    ///       "max_height_px": int,
    ///       "max_repeat": int,
    ///       "max_graphics": int,
    ///       "current_graphics": int,
    ///       "dropped_graphics": int,
    ///     }
    fn get_sixel_stats(&self) -> PyResult<HashMap<String, usize>> {
        let (limits, max_graphics, current_graphics, dropped_graphics) = self.inner.sixel_stats();
        let mut stats = HashMap::new();
        stats.insert("max_width_px".to_string(), limits.max_width);
        stats.insert("max_height_px".to_string(), limits.max_height);
        stats.insert("max_repeat".to_string(), limits.max_repeat);
        stats.insert("max_graphics".to_string(), max_graphics);
        stats.insert("current_graphics".to_string(), current_graphics);
        stats.insert("dropped_graphics".to_string(), dropped_graphics);
        Ok(stats)
    }

    /// Enable or disable tmux control mode
    ///
    /// When enabled, incoming data is parsed for tmux control protocol messages
    /// instead of being processed as raw terminal output. This allows the terminal
    /// to act as a tmux control mode client.
    ///
    /// Args:
    ///     enabled: True to enable control mode, False to disable
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     term.set_tmux_control_mode(True)
    ///     # Now the terminal will parse tmux control protocol messages
    ///     ```
    fn set_tmux_control_mode(&mut self, enabled: bool) -> PyResult<()> {
        self.inner.set_tmux_control_mode(enabled);
        Ok(())
    }

    /// Check if tmux control mode is enabled
    ///
    /// Returns:
    ///     True if control mode is enabled, False otherwise
    fn is_tmux_control_mode(&self) -> PyResult<bool> {
        Ok(self.inner.is_tmux_control_mode())
    }

    /// Get tmux control protocol notifications
    ///
    /// Returns a list of all pending tmux control protocol notifications.
    /// This does not consume the notifications. Use drain_tmux_notifications()
    /// to consume them.
    ///
    /// Returns:
    ///     List of TmuxNotification objects
    fn get_tmux_notifications(&self) -> PyResult<Vec<super::types::PyTmuxNotification>> {
        Ok(self
            .inner
            .tmux_notifications()
            .iter()
            .map(|n| n.into())
            .collect())
    }

    /// Drain and return tmux control protocol notifications
    ///
    /// Returns all pending notifications and clears the notification buffer.
    ///
    /// Returns:
    ///     List of TmuxNotification objects
    fn drain_tmux_notifications(&mut self) -> PyResult<Vec<super::types::PyTmuxNotification>> {
        Ok(self
            .inner
            .drain_tmux_notifications()
            .iter()
            .map(|n| n.into())
            .collect())
    }

    /// Check if there are pending tmux control protocol notifications
    ///
    /// Returns:
    ///     True if there are pending notifications, False otherwise
    fn has_tmux_notifications(&self) -> PyResult<bool> {
        Ok(self.inner.has_tmux_notifications())
    }

    /// Clear the tmux control protocol notifications buffer
    fn clear_tmux_notifications(&mut self) -> PyResult<()> {
        self.inner.clear_tmux_notifications();
        Ok(())
    }

    // ========== TUI App Support Methods ==========

    /// Get all dirty row numbers
    ///
    /// Returns a sorted list of 0-indexed row numbers that have been modified
    /// since the last mark_clean() call.
    fn get_dirty_rows(&self) -> PyResult<Vec<usize>> {
        Ok(self.inner.get_dirty_rows())
    }

    /// Get the dirty region bounds
    ///
    /// Returns:
    ///     Tuple of (first_row, last_row) inclusive, or None if no rows are dirty
    fn get_dirty_region(&self) -> PyResult<Option<(usize, usize)>> {
        Ok(self.inner.get_dirty_region())
    }

    /// Mark all rows as clean (clear dirty tracking)
    fn mark_clean(&mut self) -> PyResult<()> {
        self.inner.mark_clean();
        Ok(())
    }

    /// Mark a specific row as dirty
    fn mark_row_dirty(&mut self, row: usize) -> PyResult<()> {
        self.inner.mark_row_dirty(row);
        Ok(())
    }

    /// Drain all pending bell events
    ///
    /// Returns and clears the buffer of bell events.
    /// Each event is a string: 'visual', 'warning:<volume>', or 'margin:<volume>'
    fn drain_bell_events(&mut self) -> PyResult<Vec<String>> {
        use crate::terminal::BellEvent;
        Ok(self.inner.drain_bell_events().iter().map(|e| match e {
            BellEvent::VisualBell => "visual".to_string(),
            BellEvent::WarningBell(vol) => format!("warning:{}", vol),
            BellEvent::MarginBell(vol) => format!("margin:{}", vol),
        }).collect())
    }

    /// Drain all pending terminal events
    ///
    /// Returns and clears the buffer of terminal events.
    /// Events are returned as dictionaries with 'type' and additional fields.
    fn poll_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        use crate::terminal::TerminalEvent;
        let events = self.inner.poll_events();
        Ok(events.iter().map(|e| {
            let mut map = HashMap::new();
            match e {
                TerminalEvent::BellRang(bell) => {
                    map.insert("type".to_string(), "bell".to_string());
                    match bell {
                        crate::terminal::BellEvent::VisualBell => {
                            map.insert("bell_type".to_string(), "visual".to_string());
                        }
                        crate::terminal::BellEvent::WarningBell(vol) => {
                            map.insert("bell_type".to_string(), "warning".to_string());
                            map.insert("volume".to_string(), vol.to_string());
                        }
                        crate::terminal::BellEvent::MarginBell(vol) => {
                            map.insert("bell_type".to_string(), "margin".to_string());
                            map.insert("volume".to_string(), vol.to_string());
                        }
                    }
                }
                TerminalEvent::TitleChanged(title) => {
                    map.insert("type".to_string(), "title_changed".to_string());
                    map.insert("title".to_string(), title.clone());
                }
                TerminalEvent::SizeChanged(cols, rows) => {
                    map.insert("type".to_string(), "size_changed".to_string());
                    map.insert("cols".to_string(), cols.to_string());
                    map.insert("rows".to_string(), rows.to_string());
                }
                TerminalEvent::ModeChanged(mode, enabled) => {
                    map.insert("type".to_string(), "mode_changed".to_string());
                    map.insert("mode".to_string(), mode.clone());
                    map.insert("enabled".to_string(), enabled.to_string());
                }
                TerminalEvent::GraphicsAdded(row) => {
                    map.insert("type".to_string(), "graphics_added".to_string());
                    map.insert("row".to_string(), row.to_string());
                }
                TerminalEvent::HyperlinkAdded(url) => {
                    map.insert("type".to_string(), "hyperlink_added".to_string());
                    map.insert("url".to_string(), url.clone());
                }
                TerminalEvent::DirtyRegion(first, last) => {
                    map.insert("type".to_string(), "dirty_region".to_string());
                    map.insert("first_row".to_string(), first.to_string());
                    map.insert("last_row".to_string(), last.to_string());
                }
            }
            map
        }).collect())
    }

    /// Get auto-wrap mode (DECAWM)
    fn auto_wrap_mode(&self) -> PyResult<bool> {
        Ok(self.inner.auto_wrap_mode())
    }

    /// Get origin mode (DECOM)
    fn origin_mode(&self) -> PyResult<bool> {
        Ok(self.inner.origin_mode())
    }

    /// Get application cursor mode
    fn application_cursor(&self) -> PyResult<bool> {
        Ok(self.inner.application_cursor())
    }

    /// Get current scroll region
    ///
    /// Returns:
    ///     Tuple of (top, bottom) - 0-indexed, inclusive
    fn scroll_region(&self) -> PyResult<(usize, usize)> {
        Ok(self.inner.scroll_region())
    }

    /// Get left/right margins if enabled
    ///
    /// Returns:
    ///     Tuple of (left, right) if DECLRMM is enabled, None otherwise
    fn left_right_margins(&self) -> PyResult<Option<(usize, usize)>> {
        Ok(self.inner.left_right_margins())
    }

    /// Get an ANSI palette color by index (0-15)
    fn get_ansi_color(&self, index: u8) -> PyResult<Option<(u8, u8, u8)>> {
        use crate::color::Color;
        if let Some(color) = self.inner.get_ansi_color(index) {
            match color {
                Color::Rgb(r, g, b) => Ok(Some((r, g, b))),
                Color::Named(_) => Ok(None), // Named colors don't have RGB values
                Color::Indexed(_) => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    /// Get the entire ANSI color palette (colors 0-15)
    ///
    /// Returns:
    ///     List of 16 RGB tuples (r, g, b)
    fn get_ansi_palette(&self) -> PyResult<Vec<(u8, u8, u8)>> {
        use crate::color::Color;
        let palette = self.inner.get_ansi_palette();
        Ok(palette.iter().map(|c| match c {
            Color::Rgb(r, g, b) => (*r, *g, *b),
            _ => (0, 0, 0), // Fallback for non-RGB colors
        }).collect())
    }

    /// Get all tab stop positions
    fn get_tab_stops(&self) -> PyResult<Vec<usize>> {
        Ok(self.inner.get_tab_stops())
    }

    /// Set a tab stop at the specified column
    fn set_tab_stop(&mut self, col: usize) -> PyResult<()> {
        self.inner.set_tab_stop(col);
        Ok(())
    }

    /// Clear a tab stop at the specified column
    fn clear_tab_stop(&mut self, col: usize) -> PyResult<()> {
        self.inner.clear_tab_stop(col);
        Ok(())
    }

    /// Clear all tab stops
    fn clear_all_tab_stops(&mut self) -> PyResult<()> {
        self.inner.clear_all_tab_stops();
        Ok(())
    }

    /// Get all hyperlinks with their positions
    ///
    /// Returns:
    ///     List of dictionaries with 'url' (string), 'positions' (list of (col, row) tuples), and optional 'id' (string)
    fn get_all_hyperlinks(&self) -> PyResult<Vec<(String, Vec<(usize, usize)>, Option<String>)>> {
        let links = self.inner.get_all_hyperlinks();
        Ok(links.iter().map(|link| {
            (link.url.clone(), link.positions.clone(), link.id.clone())
        }).collect())
    }

    /// Get a rectangular region of the screen
    ///
    /// Returns cells in rectangle bounded by (top, left) to (bottom, right) inclusive.
    /// Returns list of rows, where each row is a list of Cell dictionaries.
    fn get_rectangle(&self, top: usize, left: usize, bottom: usize, right: usize)
        -> PyResult<Vec<Vec<HashMap<String, String>>>> {
        let cells = self.inner.get_rectangle(top, left, bottom, right);
        Ok(cells.iter().map(|row| {
            row.iter().map(|cell| {
                let mut map = HashMap::new();
                map.insert("char".to_string(), cell.c.to_string());
                map.insert("width".to_string(), cell.width.to_string());
                map
            }).collect()
        }).collect())
    }

    /// Fill a rectangle with a character
    fn fill_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize, ch: char)
        -> PyResult<()> {
        self.inner.fill_rectangle(top, left, bottom, right, ch);
        Ok(())
    }

    /// Erase a rectangle
    fn erase_rectangle(&mut self, top: usize, left: usize, bottom: usize, right: usize)
        -> PyResult<()> {
        self.inner.erase_rectangle(top, left, bottom, right);
        Ok(())
    }

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
    fn search(&self, query: &str, case_sensitive: bool) -> PyResult<Vec<super::types::PySearchMatch>> {
        let matches = self.inner.search(query, case_sensitive);
        Ok(matches.iter().map(|m| super::types::PySearchMatch {
            row: m.row,
            col: m.col,
            length: m.length,
            text: m.text.clone(),
        }).collect())
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
    fn search_scrollback(&self, query: &str, case_sensitive: bool, max_lines: Option<usize>)
        -> PyResult<Vec<super::types::PySearchMatch>> {
        let matches = self.inner.search_scrollback(query, case_sensitive, max_lines);
        Ok(matches.iter().map(|m| super::types::PySearchMatch {
            row: m.row,
            col: m.col,
            length: m.length,
            text: m.text.clone(),
        }).collect())
    }

    // === Content Detection Methods ===

    /// Detect URLs in the visible screen
    ///
    /// Returns:
    ///     List of DetectedItem objects for URLs
    fn detect_urls(&self) -> PyResult<Vec<super::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_urls();
        Ok(items.iter().map(|item| match item {
            DetectedItem::Url(text, row, col) => super::types::PyDetectedItem {
                item_type: "url".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: None,
            },
            _ => unreachable!(),
        }).collect())
    }

    /// Detect file paths in the visible screen
    ///
    /// Returns:
    ///     List of DetectedItem objects for file paths
    fn detect_file_paths(&self) -> PyResult<Vec<super::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_file_paths();
        Ok(items.iter().map(|item| match item {
            DetectedItem::FilePath(text, row, col, line_num) => super::types::PyDetectedItem {
                item_type: "filepath".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: *line_num,
            },
            _ => unreachable!(),
        }).collect())
    }

    /// Detect semantic items (URLs, file paths, git hashes, IPs, emails)
    ///
    /// Returns:
    ///     List of all detected semantic items
    fn detect_semantic_items(&self) -> PyResult<Vec<super::types::PyDetectedItem>> {
        use crate::terminal::DetectedItem;
        let items = self.inner.detect_semantic_items();
        Ok(items.iter().map(|item| match item {
            DetectedItem::Url(text, row, col) => super::types::PyDetectedItem {
                item_type: "url".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: None,
            },
            DetectedItem::FilePath(text, row, col, line_num) => super::types::PyDetectedItem {
                item_type: "filepath".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: *line_num,
            },
            DetectedItem::GitHash(text, row, col) => super::types::PyDetectedItem {
                item_type: "git_hash".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: None,
            },
            DetectedItem::IpAddress(text, row, col) => super::types::PyDetectedItem {
                item_type: "ip".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: None,
            },
            DetectedItem::Email(text, row, col) => super::types::PyDetectedItem {
                item_type: "email".to_string(),
                text: text.clone(),
                row: *row,
                col: *col,
                line_number: None,
            },
        }).collect())
    }

    // === Selection Management ===

    /// Set the current selection
    ///
    /// Args:
    ///     start: Start position (col, row) tuple
    ///     end: End position (col, row) tuple
    ///     mode: Selection mode: "character", "line", or "block"
    fn set_selection(&mut self, start: (usize, usize), end: (usize, usize), mode: &str)
        -> PyResult<()> {
        use crate::terminal::SelectionMode;
        let sel_mode = match mode {
            "character" => SelectionMode::Character,
            "line" => SelectionMode::Line,
            "block" => SelectionMode::Block,
            _ => return Err(PyValueError::new_err("Invalid selection mode")),
        };
        self.inner.set_selection(start, end, sel_mode);
        Ok(())
    }

    /// Get the current selection
    ///
    /// Returns:
    ///     Selection object or None if no selection
    fn get_selection(&self) -> PyResult<Option<super::types::PySelection>> {
        if let Some(sel) = self.inner.get_selection() {
            let mode_str = match sel.mode {
                crate::terminal::SelectionMode::Character => "character",
                crate::terminal::SelectionMode::Line => "line",
                crate::terminal::SelectionMode::Block => "block",
            };
            Ok(Some(super::types::PySelection {
                start: sel.start,
                end: sel.end,
                mode: mode_str.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Get the text content of the current selection
    ///
    /// Returns:
    ///     Selected text as string, or None if no selection
    fn get_selected_text(&self) -> PyResult<Option<String>> {
        Ok(self.inner.get_selected_text())
    }

    /// Select the word at the given position
    ///
    /// Args:
    ///     col: Column index
    ///     row: Row index
    fn select_word_at(&mut self, col: usize, row: usize) -> PyResult<()> {
        self.inner.select_word_at(col, row);
        Ok(())
    }

    /// Select the entire line at the given row
    ///
    /// Args:
    ///     row: Row index
    fn select_line(&mut self, row: usize) -> PyResult<()> {
        self.inner.select_line(row);
        Ok(())
    }

    /// Clear the current selection
    fn clear_selection(&mut self) -> PyResult<()> {
        self.inner.clear_selection();
        Ok(())
    }

    // === Text Extraction ===

    /// Get text lines around a specific row (with context)
    ///
    /// Args:
    ///     row: Center row (0-based)
    ///     context_before: Number of lines before the row
    ///     context_after: Number of lines after the row
    ///
    /// Returns:
    ///     List of text lines
    fn get_line_context(&self, row: usize, context_before: usize, context_after: usize)
        -> PyResult<Vec<String>> {
        Ok(self.inner.get_line_context(row, context_before, context_after))
    }

    /// Get the paragraph at the given position
    ///
    /// A paragraph is defined as consecutive non-empty lines.
    ///
    /// Args:
    ///     row: Row index
    ///
    /// Returns:
    ///     Paragraph text as string
    fn get_paragraph_at(&self, row: usize) -> PyResult<String> {
        Ok(self.inner.get_paragraph_at(row))
    }

    // === Scrollback Operations ===

    /// Export scrollback to various formats
    ///
    /// Args:
    ///     format: Export format: "plain", "html", or "ansi"
    ///     max_lines: Maximum number of scrollback lines to export (None = all)
    ///
    /// Returns:
    ///     Exported content as string
    #[pyo3(signature = (format="plain", max_lines=None))]
    fn export_scrollback(&self, format: &str, max_lines: Option<usize>) -> PyResult<String> {
        use crate::terminal::ExportFormat;
        let export_format = match format {
            "plain" => ExportFormat::Plain,
            "html" => ExportFormat::Html,
            "ansi" => ExportFormat::Ansi,
            _ => return Err(PyValueError::new_err("Invalid export format")),
        };
        Ok(self.inner.export_scrollback(export_format, max_lines))
    }

    /// Get scrollback statistics
    ///
    /// Returns:
    ///     ScrollbackStats object with total lines, memory usage, and wrap status
    fn scrollback_stats(&self) -> PyResult<super::types::PyScrollbackStats> {
        let stats = self.inner.scrollback_stats();
        Ok(super::types::PyScrollbackStats {
            total_lines: stats.total_lines,
            memory_bytes: stats.memory_bytes,
            has_wrapped: stats.has_wrapped,
        })
    }

    // === Bookmark Methods ===

    /// Add a bookmark at the given scrollback row
    ///
    /// Args:
    ///     row: Row index (negative for scrollback, 0+ for visible screen)
    ///     label: Optional label for the bookmark
    ///
    /// Returns:
    ///     Bookmark ID
    #[pyo3(signature = (row, label=None))]
    fn add_bookmark(&mut self, row: isize, label: Option<String>) -> PyResult<usize> {
        Ok(self.inner.add_bookmark(row, label))
    }

    /// Get all bookmarks
    ///
    /// Returns:
    ///     List of Bookmark objects
    fn get_bookmarks(&self) -> PyResult<Vec<super::types::PyBookmark>> {
        let bookmarks = self.inner.get_bookmarks();
        Ok(bookmarks.iter().map(|b| super::types::PyBookmark {
            id: b.id,
            row: b.row,
            label: b.label.clone(),
        }).collect())
    }

    /// Remove a bookmark by ID
    ///
    /// Args:
    ///     id: Bookmark ID
    ///
    /// Returns:
    ///     True if bookmark was removed, False if not found
    fn remove_bookmark(&mut self, id: usize) -> PyResult<bool> {
        Ok(self.inner.remove_bookmark(id))
    }

    /// Clear all bookmarks
    fn clear_bookmarks(&mut self) -> PyResult<()> {
        self.inner.clear_bookmarks();
        Ok(())
    }

    // === Feature 7: Performance Metrics ===

    /// Get current performance metrics
    fn get_performance_metrics(&self) -> PyResult<super::types::PyPerformanceMetrics> {
        let m = self.inner.get_performance_metrics();
        Ok(super::types::PyPerformanceMetrics {
            frames_rendered: m.frames_rendered,
            cells_updated: m.cells_updated,
            bytes_processed: m.bytes_processed,
            total_processing_us: m.total_processing_us,
            peak_frame_us: m.peak_frame_us,
            scroll_count: m.scroll_count,
            wrap_count: m.wrap_count,
            escape_sequences: m.escape_sequences,
        })
    }

    /// Reset performance metrics
    fn reset_performance_metrics(&mut self) -> PyResult<()> {
        self.inner.reset_performance_metrics();
        Ok(())
    }

    /// Record a frame timing
    fn record_frame_timing(&mut self, processing_us: u64, cells_updated: usize, bytes_processed: usize) -> PyResult<()> {
        self.inner.record_frame_timing(processing_us, cells_updated, bytes_processed);
        Ok(())
    }

    /// Get recent frame timings
    #[pyo3(signature = (count=None))]
    fn get_frame_timings(&self, count: Option<usize>) -> PyResult<Vec<super::types::PyFrameTiming>> {
        let timings = self.inner.get_frame_timings(count);
        Ok(timings.iter().map(|t| super::types::PyFrameTiming {
            frame_number: t.frame_number,
            processing_us: t.processing_us,
            cells_updated: t.cells_updated,
            bytes_processed: t.bytes_processed,
        }).collect())
    }

    /// Get average frame time in microseconds
    fn get_average_frame_time(&self) -> PyResult<u64> {
        Ok(self.inner.get_average_frame_time())
    }

    /// Get frames per second
    fn get_fps(&self) -> PyResult<f64> {
        Ok(self.inner.get_fps())
    }

    // === Feature 8: Advanced Color Operations ===

    /// Convert RGB to HSV
    fn rgb_to_hsv_color(&self, r: u8, g: u8, b: u8) -> PyResult<super::types::PyColorHSV> {
        let hsv = self.inner.rgb_to_hsv_color(r, g, b);
        Ok(super::types::PyColorHSV { h: hsv.h, s: hsv.s, v: hsv.v })
    }

    /// Convert HSV to RGB
    fn hsv_to_rgb_color(&self, h: f32, s: f32, v: f32) -> PyResult<(u8, u8, u8)> {
        let hsv = crate::terminal::ColorHSV { h, s, v };
        Ok(self.inner.hsv_to_rgb_color(hsv))
    }

    /// Convert RGB to HSL
    fn rgb_to_hsl_color(&self, r: u8, g: u8, b: u8) -> PyResult<super::types::PyColorHSL> {
        let hsl = self.inner.rgb_to_hsl_color(r, g, b);
        Ok(super::types::PyColorHSL { h: hsl.h, s: hsl.s, l: hsl.l })
    }

    /// Convert HSL to RGB
    fn hsl_to_rgb_color(&self, h: f32, s: f32, l: f32) -> PyResult<(u8, u8, u8)> {
        let hsl = crate::terminal::ColorHSL { h, s, l };
        Ok(self.inner.hsl_to_rgb_color(hsl))
    }

    /// Generate a color palette
    ///
    /// Args:
    ///     r, g, b: Base color RGB values
    ///     mode: Theme mode (complementary, analogous, triadic, tetradic, split_complementary, monochromatic)
    fn generate_color_palette(&self, r: u8, g: u8, b: u8, mode: &str) -> PyResult<super::types::PyColorPalette> {
        use crate::terminal::ThemeMode;
        let theme_mode = match mode {
            "complementary" => ThemeMode::Complementary,
            "analogous" => ThemeMode::Analogous,
            "triadic" => ThemeMode::Triadic,
            "tetradic" => ThemeMode::Tetradic,
            "split_complementary" => ThemeMode::SplitComplementary,
            "monochromatic" => ThemeMode::Monochromatic,
            _ => return Err(PyValueError::new_err("Invalid theme mode")),
        };

        let palette = self.inner.generate_color_palette(r, g, b, theme_mode);
        Ok(super::types::PyColorPalette {
            base: palette.base,
            colors: palette.colors,
            mode: mode.to_string(),
        })
    }

    /// Calculate color distance
    fn color_distance(&self, r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> PyResult<f64> {
        Ok(self.inner.color_distance(r1, g1, b1, r2, g2, b2))
    }

    // === Feature 9: Line Wrapping Utilities ===

    /// Join wrapped lines starting from a given row
    fn join_wrapped_lines(&self, start_row: usize) -> PyResult<Option<super::types::PyJoinedLines>> {
        if let Some(joined) = self.inner.join_wrapped_lines(start_row) {
            Ok(Some(super::types::PyJoinedLines {
                text: joined.text,
                start_row: joined.start_row,
                end_row: joined.end_row,
                lines_joined: joined.lines_joined,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get all logical lines (unwrapped)
    fn get_logical_lines(&self) -> PyResult<Vec<String>> {
        Ok(self.inner.get_logical_lines())
    }

    /// Check if a row starts a new logical line
    fn is_line_start(&self, row: usize) -> PyResult<bool> {
        Ok(self.inner.is_line_start(row))
    }

    // === Feature 10: Clipboard Integration ===

    /// Add content to clipboard history
    #[pyo3(signature = (slot, content, label=None))]
    fn add_to_clipboard_history(&mut self, slot: &str, content: String, label: Option<String>) -> PyResult<()> {
        let clipboard_slot = parse_clipboard_slot(slot)?;
        self.inner.add_to_clipboard_history(clipboard_slot, content, label);
        Ok(())
    }

    /// Get clipboard history for a slot
    fn get_clipboard_history(&self, slot: &str) -> PyResult<Vec<super::types::PyClipboardEntry>> {
        let clipboard_slot = parse_clipboard_slot(slot)?;
        let history = self.inner.get_clipboard_history(clipboard_slot);
        Ok(history.iter().map(|e| super::types::PyClipboardEntry {
            content: e.content.clone(),
            timestamp: e.timestamp,
            label: e.label.clone(),
        }).collect())
    }

    /// Get the most recent clipboard entry
    fn get_latest_clipboard(&self, slot: &str) -> PyResult<Option<super::types::PyClipboardEntry>> {
        let clipboard_slot = parse_clipboard_slot(slot)?;
        if let Some(entry) = self.inner.get_latest_clipboard(clipboard_slot) {
            Ok(Some(super::types::PyClipboardEntry {
                content: entry.content,
                timestamp: entry.timestamp,
                label: entry.label,
            }))
        } else {
            Ok(None)
        }
    }

    /// Clear clipboard history for a slot
    fn clear_clipboard_history(&mut self, slot: &str) -> PyResult<()> {
        let clipboard_slot = parse_clipboard_slot(slot)?;
        self.inner.clear_clipboard_history(clipboard_slot);
        Ok(())
    }

    /// Clear all clipboard history
    fn clear_all_clipboard_history(&mut self) -> PyResult<()> {
        self.inner.clear_all_clipboard_history();
        Ok(())
    }

    /// Set clipboard content with slot
    #[pyo3(signature = (content, slot=None))]
    fn set_clipboard_with_slot(&mut self, content: String, slot: Option<String>) -> PyResult<()> {
        let clipboard_slot = slot.as_ref().map(|s| parse_clipboard_slot(s)).transpose()?;
        self.inner.set_clipboard_with_slot(content, clipboard_slot);
        Ok(())
    }

    /// Get clipboard content from slot
    #[pyo3(signature = (slot=None))]
    fn get_clipboard_from_slot(&self, slot: Option<String>) -> PyResult<Option<String>> {
        let clipboard_slot = slot.as_ref().map(|s| parse_clipboard_slot(s)).transpose()?;
        Ok(self.inner.get_clipboard_from_slot(clipboard_slot))
    }

    /// Search clipboard history
    #[pyo3(signature = (query, slot=None))]
    fn search_clipboard_history(&self, query: &str, slot: Option<String>) -> PyResult<Vec<super::types::PyClipboardEntry>> {
        let clipboard_slot = slot.as_ref().map(|s| parse_clipboard_slot(s)).transpose()?;
        let results = self.inner.search_clipboard_history(query, clipboard_slot);
        Ok(results.iter().map(|e| super::types::PyClipboardEntry {
            content: e.content.clone(),
            timestamp: e.timestamp,
            label: e.label.clone(),
        }).collect())
    }
}

/// Helper function to parse clipboard slot from string
fn parse_clipboard_slot(slot: &str) -> PyResult<crate::terminal::ClipboardSlot> {
    use crate::terminal::ClipboardSlot;
    match slot.to_lowercase().as_str() {
        "primary" => Ok(ClipboardSlot::Primary),
        "clipboard" => Ok(ClipboardSlot::Clipboard),
        "selection" => Ok(ClipboardSlot::Selection),
        s if s.starts_with("custom") => {
            if let Some(num_str) = s.strip_prefix("custom") {
                if let Ok(num) = num_str.parse::<u8>() {
                    if num <= 9 {
                        return Ok(ClipboardSlot::Custom(num));
                    }
                }
            }
            Err(PyValueError::new_err("Invalid custom clipboard slot (use custom0-custom9)"))
        }
        _ => Err(PyValueError::new_err("Invalid clipboard slot")),
    }
}
