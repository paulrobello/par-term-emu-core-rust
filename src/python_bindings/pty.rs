//! Python wrapper for PtySession - terminal with PTY support
//!
//! This module provides the PyPtyTerminal struct, a Python-facing wrapper around
//! the Rust PtySession implementation. It enables interactive terminal sessions
//! with pseudo-terminal (PTY) support, including process spawning, input/output
//! handling, and advanced terminal features.
//!
//! The PyPtyTerminal struct provides:
//! - Process spawning with environment and working directory configuration
//! - Non-blocking PTY communication for interactive shells
//! - Terminal content queries and snapshots
//! - Advanced text selection and analysis utilities
//! - Graphics (Sixel) support with rendering options
//! - Shell integration (OSC 133) state tracking
//! - Clipboard, keyboard, and mouse protocol support
//! - Screenshot generation in multiple formats
//! - Buffer statistics and content search capabilities

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::color::Color;
use crate::pty_session;

use super::conversions::parse_sixel_mode;
use super::enums::PyCursorStyle;
use super::types::{LineCellData, PyAttributes, PyGraphic, PyScreenSnapshot, PyShellIntegration};

/// Python wrapper for PtySession - a terminal with PTY support
#[pyclass(name = "PtyTerminal", unsendable)]
pub struct PyPtyTerminal {
    inner: pty_session::PtySession,
}

#[pymethods]
impl PyPtyTerminal {
    /// Create a new PTY terminal with the specified dimensions
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
            inner: pty_session::PtySession::new(cols, rows, scrollback),
        })
    }

    /// Spawn a shell process (auto-detected from environment)
    ///
    /// On Unix: Uses $SHELL or defaults to /bin/bash
    /// On Windows: Uses %COMSPEC% or defaults to cmd.exe
    ///
    /// Args:
    ///     env: Optional dictionary of environment variables to set for the shell.
    ///          These are passed directly to the child process without modifying
    ///          the parent process environment (safe for multi-threaded apps).
    ///     cwd: Optional working directory path for the shell.
    #[pyo3(signature = (env=None, cwd=None))]
    fn spawn_shell(
        &mut self,
        env: Option<HashMap<String, String>>,
        cwd: Option<String>,
    ) -> PyResult<()> {
        self.inner
            .spawn_shell_with_env(env.as_ref(), cwd.as_deref())?;
        Ok(())
    }

    /// Spawn a process with the specified command and arguments
    ///
    /// Args:
    ///     command: The command to execute
    ///     args: Optional list of command-line arguments
    ///     env: Optional dictionary of environment variables
    ///     cwd: Optional working directory path
    #[pyo3(signature = (command, args=None, env=None, cwd=None))]
    fn spawn(
        &mut self,
        command: &str,
        args: Option<Vec<String>>,
        env: Option<HashMap<String, String>>,
        cwd: Option<String>,
    ) -> PyResult<()> {
        // Set environment variables if provided
        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                self.inner.set_env(&key, &value);
            }
        }

        // Set working directory if provided
        if let Some(cwd_path) = cwd {
            self.inner.set_cwd(std::path::Path::new(&cwd_path));
        }

        // Convert args to &[&str]
        let args_refs: Vec<&str> = args
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();

        self.inner.spawn(command, &args_refs)?;
        Ok(())
    }

    /// Write data to the PTY (send to the child process)
    ///
    /// Args:
    ///     data: Bytes to write
    fn write(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.write(data)?;
        Ok(())
    }

    /// Write a string to the PTY (convenience method)
    ///
    /// Args:
    ///     s: String to write
    fn write_str(&mut self, s: &str) -> PyResult<()> {
        self.inner.write_str(s)?;
        Ok(())
    }

    /// Resize the PTY and terminal
    ///
    /// Sends SIGWINCH to the child process
    ///
    /// Args:
    ///     cols: New number of columns
    ///     rows: New number of rows
    fn resize(&mut self, cols: u16, rows: u16) -> PyResult<()> {
        if cols == 0 || rows == 0 {
            return Err(PyValueError::new_err("Dimensions must be greater than 0"));
        }
        self.inner.resize(cols, rows)?;
        Ok(())
    }

    /// Resize the PTY, including pixel dimensions
    ///
    /// Args:
    ///     cols: New columns
    ///     rows: New rows
    ///     pixel_width: Text area width in pixels
    ///     pixel_height: Text area height in pixels
    #[pyo3(signature = (cols, rows, pixel_width, pixel_height))]
    fn resize_pixels(
        &mut self,
        cols: u16,
        rows: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> PyResult<()> {
        if cols == 0 || rows == 0 {
            return Err(PyValueError::new_err("Dimensions must be greater than 0"));
        }
        self.inner
            .resize_with_pixels(cols, rows, pixel_width, pixel_height)?;
        Ok(())
    }

    /// Send a resize pulse (SIGWINCH) with the current size
    ///
    /// This re-sends SIGWINCH to the child process with the same dimensions.
    /// Useful for forcing applications like tmux to recalculate their layout.
    fn send_resize_pulse(&mut self) -> PyResult<()> {
        let (cols, rows) = self.inner.size();
        self.inner.resize(cols as u16, rows as u16)?;
        Ok(())
    }

    /// Check if the process is still running
    ///
    /// Returns:
    ///     True if the process is running
    fn is_running(&self) -> PyResult<bool> {
        Ok(self.inner.is_running())
    }

    /// Wait for the process to exit and return its exit code
    ///
    /// This blocks until the process exits
    ///
    /// Returns:
    ///     Exit code of the process
    fn wait(&mut self) -> PyResult<i32> {
        let code = self.inner.wait()?;
        Ok(code)
    }

    /// Try to get the exit status without blocking
    ///
    /// Returns:
    ///     Exit code if the process has exited, None otherwise
    fn try_wait(&mut self) -> PyResult<Option<i32>> {
        let status = self.inner.try_wait()?;
        Ok(status)
    }

    /// Kill the process
    fn kill(&mut self) -> PyResult<()> {
        self.inner.kill()?;
        Ok(())
    }

    // Terminal query methods

    /// Get the terminal content as a string
    ///
    /// Returns:
    ///     String representation of the terminal buffer
    fn content(&self) -> PyResult<String> {
        Ok(self.inner.content())
    }

    /// Get the terminal title
    ///
    /// Returns the title string set by OSC 0, 1, or 2 sequences.
    ///
    /// Returns:
    ///     Current terminal title string
    fn title(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let title = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.title().to_string()
        } else {
            String::new()
        };
        Ok(title)
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
    ///     font_path: Path to TTF/OTF font file. Default: None (use embedded JetBrains Mono)
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
    ///     bold_brightening: Enable bold brightening (ANSI 0-7 -> 8-15). Default: None (use theme setting)
    ///     background_color: Background color RGB tuple. Default: None (use terminal's default background)
    ///     faint_text_alpha: Alpha multiplier for faint/dim text (0.0-1.0). Default: 0.5 (50% dimming)
    ///     minimum_contrast: Minimum contrast adjustment (0.0-1.0). Default: 0.5 (moderate contrast adjustment)
    ///
    /// Returns:
    ///     Bytes of the image in the specified format
    ///
    /// Note:
    ///     Fonts: Embedded JetBrains Mono + Noto Emoji (monochrome) are used by default.
    ///     System emoji/CJK fonts are automatically used as fallback when available.
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
        bold_brightening = None,
        background_color = None,
        faint_text_alpha = 0.5,
        minimum_contrast = 0.5
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
        bold_brightening: Option<bool>,
        background_color: Option<(u8, u8, u8)>,
        faint_text_alpha: Option<f32>,
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

        // Get theme settings from terminal (used as defaults if not explicitly provided)
        let terminal = self.inner.terminal();
        let (term_bold_brightening, term_bg_color) = if let Ok(term) = Ok::<_, ()>(terminal.lock())
        {
            (term.bold_brightening(), term.default_bg().to_rgb())
        } else {
            (false, (0, 0, 0))
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
            sixel_render_mode: parse_sixel_mode(sixel_mode)?,
            link_color,
            bold_color,
            use_bold_color: use_bold_color.unwrap_or(false),
            bold_brightening: bold_brightening.unwrap_or(term_bold_brightening),
            background_color: background_color.or(Some(term_bg_color)),
            minimum_contrast: minimum_contrast.clamp(0.0, 1.0),
            faint_text_alpha: faint_text_alpha.unwrap_or(0.5).clamp(0.0, 1.0),
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
    ///     font_path: Path to TTF/OTF font file. Default: None (use embedded JetBrains Mono)
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
    ///     bold_brightening: Enable bold brightening (ANSI 0-7 -> 8-15). Default: None (use theme setting)
    ///     background_color: Background color RGB tuple. Default: None (use terminal's default background)
    ///     faint_text_alpha: Alpha multiplier for faint/dim text (0.0-1.0). Default: 0.5 (50% dimming)
    ///     minimum_contrast: Minimum contrast adjustment (0.0-1.0). Default: 0.5 (moderate contrast adjustment)
    ///
    /// Returns:
    ///     None
    ///
    /// Note:
    ///     Fonts: Embedded JetBrains Mono + Noto Emoji (monochrome) are used by default.
    ///     System emoji/CJK fonts are automatically used as fallback when available.
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
        bold_brightening = None,
        background_color = None,
        faint_text_alpha = 0.5,
        minimum_contrast = 0.5
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
        bold_brightening: Option<bool>,
        background_color: Option<(u8, u8, u8)>,
        faint_text_alpha: Option<f32>,
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
            bold_brightening,
            background_color,
            faint_text_alpha,
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

    /// Get the cursor position
    ///
    /// Returns:
    ///     Tuple of (col, row)
    fn cursor_position(&self) -> PyResult<(usize, usize)> {
        Ok(self.inner.cursor_position())
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
        Ok(self.inner.scrollback_len())
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
    ) -> PyResult<Option<Vec<(String, (u8, u8, u8), (u8, u8, u8), PyAttributes)>>> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        let grid = term.grid();
        if let Some(line) = grid.scrollback_line(index) {
            let cells: Vec<_> = line
                .iter()
                .map(|cell| {
                    (
                        cell.get_grapheme(),
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
        Ok(self.inner.get_line(row))
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
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid().get(col, row).map(|cell| cell.c)
        } else {
            None
        };
        Ok(result)
    }

    /// Check if a line is wrapped (continues to the next line)
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     True if the line wraps to the next row, False otherwise
    fn is_line_wrapped(&self, row: usize) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid().is_line_wrapped(row)
        } else {
            false
        };
        Ok(result)
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
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid()
                .get(col, row)
                .map(|cell| cell.fg.to_rgb())
        } else {
            None
        };
        Ok(result)
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
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid()
                .get(col, row)
                .map(|cell| cell.bg.to_rgb())
        } else {
            None
        };
        Ok(result)
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
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid()
                .get(col, row)
                .and_then(|cell| cell.underline_color.map(|c| c.to_rgb()))
        } else {
            None
        };
        Ok(result)
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
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.active_grid().get(col, row).map(|cell| PyAttributes {
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
            })
        } else {
            None
        };
        Ok(result)
    }

    /// Get hyperlink URL at the specified position (OSC 8)
    ///
    /// Retrieves the URL associated with a hyperlink at the given position.
    /// Hyperlinks are created using OSC 8 sequences (e.g., `\x1b]8;;URL\x07text\x1b]8;;\x07`).
    ///
    /// Args:
    ///     col: Column position (0-based)
    ///     row: Row position (0-based)
    ///
    /// Returns:
    ///     URL string if a hyperlink exists at that position, None otherwise
    fn get_hyperlink(&self, col: usize, row: usize) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            if let Some(cell) = term.active_grid().get(col, row) {
                if let Some(id) = cell.flags.hyperlink_id {
                    term.get_hyperlink_url(id)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        Ok(result)
    }

    /// Get all cell data for a row in a single atomic operation
    ///
    /// This method retrieves all cell information for an entire row with a single lock,
    /// preventing race conditions where the PTY thread updates state between individual
    /// cell attribute reads.
    ///
    /// Args:
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     List of tuples (char, (fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), attributes) for each column,
    ///     or empty list if row is out of bounds
    fn get_line_cells(&self, row: usize) -> PyResult<LineCellData> {
        let terminal = self.inner.terminal();
        let result = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            let grid = term.active_grid();
            let rows = grid.rows();

            if row >= rows {
                Vec::new()
            } else {
                let cols = grid.cols();
                (0..cols)
                    .filter_map(|col| {
                        grid.get(col, row).map(|cell| {
                            (
                                cell.get_grapheme(),
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
                    .collect()
            }
        } else {
            Vec::new()
        };
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();

        // Get current grid (will be either primary or alternate)
        let grid = term.active_grid();
        let rows = grid.rows();
        let cols = grid.cols();

        // Get bold brightening setting
        let bold_brightening = term.bold_brightening();

        // Get ANSI palette for color resolution
        let ansi_palette = term.get_ansi_palette();

        // Helper function to resolve foreground color using the palette
        let resolve_fg_color = |color: crate::color::Color| -> (u8, u8, u8) {
            match color {
                crate::color::Color::Named(named) => {
                    // Use palette color instead of hardcoded ANSI color
                    let palette_idx = named as usize;
                    if palette_idx < 16 {
                        ansi_palette[palette_idx].to_rgb()
                    } else {
                        color.to_rgb() // Fallback to hardcoded (shouldn't happen)
                    }
                }
                crate::color::Color::Indexed(idx) if (idx as usize) < 16 => {
                    // Indexed colors 0-15 also use palette
                    ansi_palette[idx as usize].to_rgb()
                }
                _ => color.to_rgb(), // RGB and indexed 16-255 use their own values
            }
        };

        // Helper function to resolve background color using the palette
        let resolve_bg_color = |color: crate::color::Color| -> (u8, u8, u8) {
            match color {
                crate::color::Color::Named(named) => {
                    // Use palette color instead of hardcoded ANSI color
                    let palette_idx = named as usize;
                    if palette_idx < 16 {
                        ansi_palette[palette_idx].to_rgb()
                    } else {
                        color.to_rgb() // Fallback to hardcoded (shouldn't happen)
                    }
                }
                crate::color::Color::Indexed(idx) if (idx as usize) < 16 => {
                    // Indexed colors 0-15 also use palette
                    ansi_palette[idx as usize].to_rgb()
                }
                _ => color.to_rgb(), // RGB and indexed 16-255 use their own values
            }
        };

        // Capture all lines while holding terminal lock
        let mut lines = Vec::with_capacity(rows);
        let mut wrapped_lines = Vec::with_capacity(rows);
        for row in 0..rows {
            let mut line = Vec::with_capacity(cols);
            for col in 0..cols {
                if let Some(cell) = grid.get(col, row) {
                    // Apply bold brightening: if bold and color is ANSI 0-7, use bright variant 8-15
                    let mut fg = cell.fg;
                    if bold_brightening && cell.flags.bold() {
                        if let crate::color::Color::Named(named) = fg {
                            if (named as u8) < 8 {
                                // Convert normal ANSI color (0-7) to bright variant (8-15)
                                fg = crate::color::Color::Named(crate::color::NamedColor::from_u8(
                                    named as u8 + 8,
                                ));
                            }
                        }
                    }

                    line.push((
                        cell.get_grapheme(),
                        resolve_fg_color(fg),
                        resolve_bg_color(cell.bg),
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
                    line.push((
                        " ".to_string(),
                        (0, 0, 0),
                        (0, 0, 0),
                        PyAttributes::default(),
                    ));
                }
            }
            lines.push(line);
            wrapped_lines.push(grid.is_line_wrapped(row));
        }

        let cursor = term.cursor();

        // Get generation before releasing lock
        let generation = self.inner.update_generation();

        Ok(PyScreenSnapshot {
            lines,
            wrapped_lines,
            cursor_pos: (cursor.col, cursor.row),
            cursor_visible: cursor.visible,
            cursor_style: cursor.style.into(),
            is_alt_screen: term.is_alt_screen_active(),
            generation,
            size: (cols, rows),
        })
    }

    /// Get the default shell for the current platform
    ///
    /// Returns:
    ///     Path to the default shell
    #[staticmethod]
    fn get_default_shell() -> PyResult<String> {
        Ok(pty_session::PtySession::get_default_shell())
    }

    /// Get the current update generation number
    ///
    /// This number is incremented every time the terminal content changes.
    /// Useful for detecting when to redraw in event loops.
    ///
    /// Returns:
    ///     The current generation number
    fn update_generation(&self) -> PyResult<u64> {
        Ok(self.inner.update_generation())
    }

    /// Check if the terminal has been updated since a given generation
    ///
    /// Args:
    ///     last_generation: The generation number from a previous call to update_generation()
    ///
    /// Returns:
    ///     True if updates have occurred since the given generation
    fn has_updates_since(&self, last_generation: u64) -> PyResult<bool> {
        Ok(self.inner.has_updates_since(last_generation))
    }

    /// Get the current bell event count
    ///
    /// This counter increments each time the terminal receives a bell character (BEL/\\x07).
    /// Applications can poll this to detect bell events for visual bell implementations.
    ///
    /// Returns:
    ///     The total number of bell events received since terminal creation
    fn bell_count(&self) -> PyResult<u64> {
        Ok(self.inner.bell_count())
    }

    /// Get mouse tracking mode
    ///
    /// Returns:
    ///     String representing the mouse mode: "off", "x10", "normal", "button", "any"
    fn mouse_mode(&self) -> PyResult<String> {
        use crate::mouse::MouseMode;
        let terminal = self.inner.terminal();
        let mode = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            match term.mouse_mode() {
                MouseMode::Off => "off",
                MouseMode::X10 => "x10",
                MouseMode::Normal => "normal",
                MouseMode::ButtonEvent => "button",
                MouseMode::AnyEvent => "any",
            }
        } else {
            "off"
        };
        Ok(mode.to_string())
    }

    /// Check if cursor is visible
    ///
    /// Returns:
    ///     True if cursor is visible
    fn cursor_visible(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let visible = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.cursor().visible
        } else {
            false
        };
        Ok(visible)
    }

    /// Get current Kitty Keyboard Protocol flags
    ///
    /// Returns:
    ///     Current keyboard protocol flags (u16)
    ///     Flags: 1=disambiguate, 2=report events, 4=alternate keys, 8=report all, 16=associated text
    fn keyboard_flags(&self) -> PyResult<u16> {
        let terminal = self.inner.terminal();
        let flags = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.keyboard_flags()
        } else {
            0
        };
        Ok(flags)
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
        self.write(sequence.as_bytes())?;
        Ok(())
    }

    /// Query Kitty Keyboard Protocol flags (sends CSI ? u)
    ///
    /// Returns:
    ///     Query sequence sent to terminal (response will be in terminal responses)
    fn query_keyboard_flags(&mut self) -> PyResult<()> {
        self.write(b"\x1b[?u")?;
        Ok(())
    }

    /// Get insert mode (IRM - Mode 4) state
    ///
    /// Returns:
    ///     True if insert mode is enabled (characters are inserted), False if replace mode (default)
    fn insert_mode(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let mode = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.insert_mode()
        } else {
            false
        };
        Ok(mode)
    }

    /// Get line feed/new line mode (LNM - Mode 20) state
    ///
    /// Returns:
    ///     True if LNM is enabled (LF does CR+LF), False if LF only (default)
    fn line_feed_new_line_mode(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let mode = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.line_feed_new_line_mode()
        } else {
            false
        };
        Ok(mode)
    }

    /// Push current keyboard flags to stack and set new flags
    ///
    /// Args:
    ///     flags: New flags to set
    ///
    /// Sends: CSI > flags u
    fn push_keyboard_flags(&mut self, flags: u16) -> PyResult<()> {
        let sequence = format!("\x1b[>{}u", flags);
        self.write(sequence.as_bytes())?;
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
        self.write(sequence.as_bytes())?;
        Ok(())
    }

    /// Force set keyboard protocol flags directly (bypasses protocol sequences)
    ///
    /// Unlike set_keyboard_flags() which sends CSI sequences to the application,
    /// this method directly modifies the terminal's internal keyboard_flags state.
    /// Useful for resetting stuck keyboard protocol when applications fail to
    /// properly disable it on exit.
    ///
    /// Args:
    ///     flags: Keyboard protocol flags to set (0 = normal mode)
    ///
    /// Example:
    ///     >>> term.force_set_keyboard_flags(0)  # Reset to normal mode
    fn force_set_keyboard_flags(&mut self, flags: u16) -> PyResult<()> {
        let terminal = self.inner.terminal();
        let mut term = terminal.lock();
        term.set_keyboard_flags(flags);
        Ok(())
    }

    /// Get clipboard content (OSC 52)
    ///
    /// Returns:
    ///     Clipboard content as string, or None if empty
    fn clipboard(&self) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let content = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.clipboard().map(|s| s.to_string())
        } else {
            None
        };
        Ok(content)
    }

    /// Set clipboard content programmatically
    ///
    /// This bypasses OSC 52 sequences and directly sets the clipboard.
    /// Useful for integration with system clipboard or testing.
    ///
    /// Args:
    ///     content: Content to set (None to clear)
    fn set_clipboard(&mut self, content: Option<String>) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_clipboard(content);
        }
        Ok(())
    }

    /// Check if clipboard read operations are allowed
    ///
    /// Returns:
    ///     True if OSC 52 queries (ESC ] 52 ; c ; ? ST) are allowed
    fn allow_clipboard_read(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let allowed = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.allow_clipboard_read()
        } else {
            false
        };
        Ok(allowed)
    }

    /// Set whether clipboard read operations are allowed
    ///
    /// When disabled (default), OSC 52 queries are silently ignored for security.
    /// When enabled, terminal applications can query clipboard contents.
    ///
    /// Args:
    ///     allow: True to allow clipboard read, False to block (default)
    fn set_allow_clipboard_read(&mut self, allow: bool) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_allow_clipboard_read(allow);
        }
        Ok(())
    }

    /// Get default foreground color (OSC 10)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn default_fg(&self) -> PyResult<(u8, u8, u8)> {
        let terminal = self.inner.terminal();
        let color = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.default_fg().to_rgb()
        } else {
            (192, 192, 192) // Default white if lock fails
        };
        Ok(color)
    }

    /// Set default foreground color (OSC 10)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_default_fg(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_default_fg(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Query default foreground color (OSC 10)
    ///
    /// Sends OSC 10 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 10 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_fg(&mut self) -> PyResult<()> {
        self.write(b"\x1b]10;?\x1b\\")?;
        Ok(())
    }

    /// Get default background color (OSC 11)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn default_bg(&self) -> PyResult<(u8, u8, u8)> {
        let terminal = self.inner.terminal();
        let color = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.default_bg().to_rgb()
        } else {
            (0, 0, 0) // Default black if lock fails
        };
        Ok(color)
    }

    /// Set default background color (OSC 11)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_default_bg(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_default_bg(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Query default background color (OSC 11)
    ///
    /// Sends OSC 11 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 11 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_bg(&mut self) -> PyResult<()> {
        self.write(b"\x1b]11;?\x1b\\")?;
        Ok(())
    }

    /// Get cursor color (OSC 12)
    ///
    /// Returns RGB tuple (r, g, b) where each component is 0-255.
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers
    fn cursor_color(&self) -> PyResult<(u8, u8, u8)> {
        let terminal = self.inner.terminal();
        let color = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.cursor_color().to_rgb()
        } else {
            (192, 192, 192) // Default white if lock fails
        };
        Ok(color)
    }

    /// Set cursor color (OSC 12)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_cursor_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_cursor_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Query cursor color (OSC 12)
    ///
    /// Sends OSC 12 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 12 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_cursor_color(&mut self) -> PyResult<()> {
        self.write(b"\x1b]12;?\x1b\\")?;
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
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_ansi_palette_color(index, Color::Rgb(r, g, b))
                .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
        }
        Ok(())
    }

    /// Set link/hyperlink color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_link_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_link_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set bold text color (when use_bold_color is enabled)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_bold_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_bold_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set cursor guide color (vertical line following cursor)
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_cursor_guide_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_cursor_guide_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set badge color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_badge_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_badge_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set match/search highlight color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_match_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_match_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set selection background color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_selection_bg_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_selection_bg_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Set selection foreground/text color
    ///
    /// Args:
    ///     r: Red component (0-255)
    ///     g: Green component (0-255)
    ///     b: Blue component (0-255)
    fn set_selection_fg_color(&mut self, r: u8, g: u8, b: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_selection_fg_color(Color::Rgb(r, g, b));
        }
        Ok(())
    }

    /// Enable/disable custom bold color
    ///
    /// When enabled, bold text uses set_bold_color() instead of bright ANSI variant.
    ///
    /// Args:
    ///     use_bold: Whether to use custom bold color
    fn set_use_bold_color(&mut self, use_bold: bool) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_use_bold_color(use_bold);
        }
        Ok(())
    }

    /// Enable/disable bold brightening
    ///
    /// When enabled, bold text with ANSI colors 0-7 uses bright variants 8-15.
    /// This matches iTerm2's "Use Bright Bold" setting.
    ///
    /// Args:
    ///     enabled: Whether to brighten bold text with colors 0-7
    fn set_bold_brightening(&mut self, enabled: bool) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_bold_brightening(enabled);
        }
        Ok(())
    }

    /// Enable/disable custom underline color
    ///
    /// When enabled, underlined text uses a custom underline color.
    ///
    /// Args:
    ///     use_underline: Whether to use custom underline color
    fn set_use_underline_color(&mut self, use_underline: bool) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_use_underline_color(use_underline);
        }
        Ok(())
    }

    /// Get cursor style (DECSCUSR)
    ///
    /// Returns:
    ///     CursorStyle enum value
    fn cursor_style(&self) -> PyResult<PyCursorStyle> {
        let terminal = self.inner.terminal();
        let style = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.cursor().style().into()
        } else {
            PyCursorStyle::BlinkingBlock // Default if lock fails
        };
        Ok(style)
    }

    /// Set cursor style (DECSCUSR)
    ///
    /// This is equivalent to sending CSI <n> SP q escape sequence.
    ///
    /// Args:
    ///     style: CursorStyle enum value (e.g., CursorStyle.BlinkingBlock)
    fn set_cursor_style(&mut self, style: PyCursorStyle) -> PyResult<()> {
        // Process DECSCUSR escape sequence locally (CSI <n> SP q)
        // This should NOT be sent to the PTY - cursor styling is a TUI rendering concern
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
        // Process the sequence through the terminal's parser instead of sending to PTY
        let terminal = self.inner.terminal();
        let mut term = terminal.lock();
        term.process(sequence.as_bytes());
        Ok(())
    }

    /// Check if alternate screen is active
    ///
    /// Returns:
    ///     True if alternate screen is active
    fn is_alt_screen_active(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let is_alt = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.is_alt_screen_active()
        } else {
            false
        };
        Ok(is_alt)
    }

    /// Check if focus tracking is enabled
    ///
    /// Returns:
    ///     True if focus tracking is enabled (DECSET 1004)
    fn focus_tracking(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let enabled = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.focus_tracking()
        } else {
            false
        };
        Ok(enabled)
    }

    /// Get focus in event sequence
    ///
    /// Returns the escape sequence to send when terminal gains focus.
    /// Only relevant when focus tracking is enabled (CSI ? 1004 h).
    ///
    /// Returns:
    ///     Bytes for focus in event: b'\x1b[I'
    fn get_focus_in_event(&self) -> PyResult<Vec<u8>> {
        let terminal = self.inner.terminal();
        let event = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.report_focus_in()
        } else {
            Vec::new()
        };
        Ok(event)
    }

    /// Get focus out event sequence
    ///
    /// Returns the escape sequence to send when terminal loses focus.
    /// Only relevant when focus tracking is enabled (CSI ? 1004 h).
    ///
    /// Returns:
    ///     Bytes for focus out event: b'\x1b[O'
    fn get_focus_out_event(&self) -> PyResult<Vec<u8>> {
        let terminal = self.inner.terminal();
        let event = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.report_focus_out()
        } else {
            Vec::new()
        };
        Ok(event)
    }

    /// Check if bracketed paste mode is enabled
    ///
    /// Returns:
    ///     True if bracketed paste mode is enabled (DECSET 2004)
    fn bracketed_paste(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let enabled = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.bracketed_paste()
        } else {
            false
        };
        Ok(enabled)
    }

    /// Get bracketed paste start sequence
    ///
    /// Returns:
    ///     Bytes for paste start (if bracketed paste is enabled)
    fn get_paste_start(&self) -> PyResult<Vec<u8>> {
        let terminal = self.inner.terminal();
        let sequence = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.bracketed_paste_start().to_vec()
        } else {
            Vec::new()
        };
        Ok(sequence)
    }

    /// Get bracketed paste end sequence
    ///
    /// Returns:
    ///     Bytes for paste end (if bracketed paste is enabled)
    fn get_paste_end(&self) -> PyResult<Vec<u8>> {
        let terminal = self.inner.terminal();
        let sequence = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.bracketed_paste_end().to_vec()
        } else {
            Vec::new()
        };
        Ok(sequence)
    }

    /// Paste text content into terminal with bracketed paste support
    ///
    /// If bracketed paste mode is enabled, wraps the content with ESC[200~ and ESC[201~
    /// Otherwise, writes the content directly to the PTY
    ///
    /// Args:
    ///     content: String content to paste
    fn paste(&mut self, content: &str) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            // Get the paste sequences (handles bracketed paste mode)
            let start = term.bracketed_paste_start();
            let end = term.bracketed_paste_end();

            // Write start sequence if in bracketed paste mode
            if !start.is_empty() {
                self.write(start)?;
            }

            // Write the actual content
            self.write_str(content)?;

            // Write end sequence if in bracketed paste mode
            if !end.is_empty() {
                self.write(end)?;
            }
        }
        Ok(())
    }

    /// Check if synchronized updates mode is enabled (DEC 2026)
    ///
    /// Returns:
    ///     True if synchronized updates mode is enabled
    fn synchronized_updates(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let enabled = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.synchronized_updates()
        } else {
            false
        };
        Ok(enabled)
    }

    /// Flush synchronized updates (DEC 2026)
    ///
    /// When synchronized update mode is active (CSI ? 2026 h), this flushes
    /// all pending updates atomically for flicker-free rendering.
    fn flush_synchronized_updates(&mut self) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.flush_synchronized_updates();
        }
        Ok(())
    }

    // Device query response methods

    /// Drain all pending device query responses
    ///
    /// This retrieves and clears all buffered responses from device queries
    /// (DA, DSR, DECRQM, DECREQTPARM). The responses are automatically written
    /// back to the PTY by the reader thread, so this method is typically only
    /// needed for debugging or testing.
    ///
    /// Returns:
    ///     Bytes containing all pending responses
    fn drain_responses(&mut self) -> PyResult<Vec<u8>> {
        let terminal = self.inner.terminal();
        let responses = if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.drain_responses()
        } else {
            Vec::new()
        };
        Ok(responses)
    }

    /// Check if there are pending device query responses
    ///
    /// Returns:
    ///     True if there are buffered responses waiting
    fn has_pending_responses(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let has_pending = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.has_pending_responses()
        } else {
            false
        };
        Ok(has_pending)
    }

    // Notification methods (OSC 9 / OSC 777)

    /// Check if there are pending notifications
    ///
    /// Returns:
    ///     True if there are notifications waiting to be retrieved
    fn has_notifications(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let has_pending = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.has_notifications()
        } else {
            false
        };
        Ok(has_pending)
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
        let terminal = self.inner.terminal();
        let notifications = if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.take_notifications()
        } else {
            Vec::new()
        };
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

    // Progress bar methods (OSC 9;4 - ConEmu/Windows Terminal style)

    /// Get the current progress bar state
    ///
    /// Returns the progress bar state set via OSC 9;4 sequences.
    /// The progress bar has a state (hidden, normal, indeterminate, warning, error)
    /// and a percentage (0-100) for states that support it.
    ///
    /// Returns:
    ///     ProgressBar object with state and progress fields
    fn progress_bar(&self) -> PyResult<super::types::PyProgressBar> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.progress_bar().into())
    }

    /// Check if the progress bar is currently active (visible)
    ///
    /// Returns:
    ///     True if the progress bar is in any state other than Hidden
    fn has_progress(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.has_progress())
    }

    /// Get the current progress percentage (0-100)
    ///
    /// Returns the progress percentage. Only meaningful when the progress bar
    /// state is Normal, Warning, or Error.
    ///
    /// Returns:
    ///     Progress percentage (0-100)
    fn progress_value(&self) -> PyResult<u8> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.progress_value())
    }

    /// Get the current progress bar state enum
    ///
    /// Returns:
    ///     ProgressState enum value (Hidden, Normal, Indeterminate, Warning, Error)
    fn progress_state(&self) -> PyResult<super::enums::PyProgressState> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.progress_state().into())
    }

    /// Manually set the progress bar state
    ///
    /// This can be used to programmatically control the progress bar
    /// without receiving OSC 9;4 sequences.
    ///
    /// Args:
    ///     state: ProgressState enum value
    ///     progress: Progress percentage (0-100, clamped if out of range)
    fn set_progress(&mut self, state: super::enums::PyProgressState, progress: u8) -> PyResult<()> {
        let terminal = self.inner.terminal();
        let mut term = terminal.lock();
        term.set_progress(state.into(), progress);
        Ok(())
    }

    /// Clear/hide the progress bar
    ///
    /// Equivalent to receiving OSC 9;4;0 (hidden state).
    fn clear_progress(&mut self) -> PyResult<()> {
        let terminal = self.inner.terminal();
        let mut term = terminal.lock();
        term.clear_progress();
        Ok(())
    }

    /// Get a debug snapshot of the current buffer state
    ///
    /// Returns:
    ///     String containing a formatted view of the buffer
    fn debug_snapshot_buffer(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        let grid = term.active_grid();
        Ok(grid.debug_snapshot())
    }

    /// Get a debug snapshot of the grid
    ///
    /// Returns:
    ///     String containing a formatted view of the grid
    fn debug_snapshot_grid(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the primary screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the primary buffer
    fn debug_snapshot_primary(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the alternate screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the alternate buffer
    fn debug_snapshot_alt(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.alt_grid().debug_snapshot())
    }

    /// Log a debug snapshot with a label
    ///
    /// Args:
    ///     label: Description of this snapshot
    fn debug_log_snapshot(&self, label: &str) -> PyResult<()> {
        use crate::debug;
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        let grid = term.active_grid();
        let snapshot = grid.debug_snapshot();
        debug::log_buffer_snapshot(label, grid.rows(), grid.cols(), &snapshot);
        Ok(())
    }

    /// Get shell integration state
    ///
    /// Returns:
    ///     Dictionary with shell integration info
    fn shell_integration_state(&self) -> PyResult<PyShellIntegration> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        let si = term.shell_integration();
        Ok(PyShellIntegration {
            in_prompt: si.in_prompt(),
            in_command_input: si.in_command_input(),
            in_command_output: si.in_command_output(),
            current_command: si.command().map(|s| s.to_string()),
            last_exit_code: si.exit_code(),
            cwd: si.cwd().map(|s| s.to_string()),
        })
    }

    /// Get current working directory from shell integration (OSC 7)
    ///
    /// Returns the directory path reported by the shell via OSC 7 sequences,
    /// or None if no directory has been reported yet.
    ///
    /// Returns:
    ///     Optional string with current directory path
    fn current_directory(&self) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let cwd = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.current_directory().map(|s| s.to_string())
        } else {
            None
        };
        Ok(cwd)
    }

    /// Check if OSC 7 directory tracking is enabled
    ///
    /// Returns:
    ///     True if OSC 7 sequences are accepted, False otherwise
    fn accept_osc7(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let accepted = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.accept_osc7()
        } else {
            true // Default
        };
        Ok(accepted)
    }

    /// Set whether OSC 7 directory tracking sequences are accepted
    ///
    /// When disabled, OSC 7 sequences are silently ignored.
    /// When enabled (default), allows shell to report current working directory.
    ///
    /// Args:
    ///     accept: True to accept OSC 7 (default), False to ignore
    fn set_accept_osc7(&mut self, accept: bool) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_accept_osc7(accept);
        }
        Ok(())
    }

    /// Check if insecure sequence filtering is enabled
    ///
    /// Returns:
    ///     True if insecure sequences are blocked, False otherwise
    fn disable_insecure_sequences(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let disabled = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.disable_insecure_sequences()
        } else {
            false // Default
        };
        Ok(disabled)
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
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_disable_insecure_sequences(disable);
        }
        Ok(())
    }

    /// Get current debug information as a dictionary
    ///
    /// Returns:
    ///     Dictionary containing terminal state for debugging
    fn debug_info(&self) -> PyResult<HashMap<String, String>> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();

        let mut info = HashMap::new();
        let (cols, rows) = term.size();
        let cursor = term.cursor();

        info.insert("size".to_string(), format!("{}x{}", cols, rows));
        info.insert(
            "cursor_pos".to_string(),
            format!("({},{})", cursor.col, cursor.row),
        );
        info.insert("cursor_visible".to_string(), cursor.visible.to_string());
        info.insert(
            "alt_screen_active".to_string(),
            term.is_alt_screen_active().to_string(),
        );
        info.insert(
            "scrollback_len".to_string(),
            term.scrollback().len().to_string(),
        );
        info.insert("title".to_string(), term.title().to_string());
        info.insert(
            "pty_running".to_string(),
            self.inner.is_running().to_string(),
        );
        info.insert(
            "update_generation".to_string(),
            self.inner.update_generation().to_string(),
        );

        Ok(info)
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
        let terminal = self.inner.terminal();
        let graphics = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            let graphics = term.graphics_at_row(row);
            graphics.iter().map(|g| PyGraphic::from(*g)).collect()
        } else {
            Vec::new()
        };
        Ok(graphics)
    }

    /// Get total number of graphics
    ///
    /// Returns:
    ///     Total count of Sixel graphics
    fn graphics_count(&self) -> PyResult<usize> {
        let terminal = self.inner.terminal();
        let count = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.graphics_count()
        } else {
            0
        };
        Ok(count)
    }

    /// Get all graphics
    ///
    /// Returns:
    ///     List of all Sixel graphics
    fn graphics(&self) -> PyResult<Vec<PyGraphic>> {
        let terminal = self.inner.terminal();
        let graphics = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            let graphics = term.all_graphics();
            graphics.iter().map(PyGraphic::from).collect()
        } else {
            Vec::new()
        };
        Ok(graphics)
    }

    /// Clear all graphics
    fn clear_graphics(&mut self) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.clear_graphics();
        }
        Ok(())
    }

    /// Update all Kitty graphics animations and trigger refresh if frames changed
    ///
    /// This method should be called regularly (e.g., 60Hz) to advance animation frames.
    /// It returns a list of image IDs whose frames changed, allowing frontends to
    /// selectively refresh only graphics that were updated.
    ///
    /// Returns:
    ///     List of image IDs that changed frames
    fn update_animations(&mut self) -> PyResult<Vec<u32>> {
        let terminal = self.inner.terminal();
        let changed = if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.update_animations()
        } else {
            Vec::new()
        };
        Ok(changed)
    }

    fn __repr__(&self) -> PyResult<String> {
        let (cols, rows) = self.inner.size();
        let running = if self.inner.is_running() {
            "running"
        } else {
            "stopped"
        };
        Ok(format!(
            "PtyTerminal(cols={}, rows={}, status={})",
            cols, rows, running
        ))
    }

    fn __str__(&self) -> PyResult<String> {
        Ok(self.inner.content())
    }

    // Context manager support
    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        Ok(slf)
    }

    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        // Kill process if still running
        if self.inner.is_running() {
            let _ = self.inner.kill();
        }
        Ok(false) // Don't suppress exceptions
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.get_word_at(col, row, word_chars))
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.get_url_at(col, row))
    }

    /// Get full logical line following wrapping
    ///
    /// Args:
    ///     row: Row position (0-indexed)
    ///
    /// Returns:
    ///     Complete unwrapped line or None if row is invalid
    fn get_line_unwrapped(&self, row: usize) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.get_line_unwrapped(row))
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.select_word(col, row, word_chars))
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.find_text(pattern, case_sensitive))
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.find_next(pattern, from_col, from_row, case_sensitive))
    }

    // ========== Buffer Statistics ==========

    /// Get terminal statistics
    ///
    /// Returns:
    ///     Dictionary with statistics: cols, rows, scrollback_lines, total_cells,
    ///     non_whitespace_lines, graphics_count, estimated_memory_bytes
    fn get_stats(&self) -> PyResult<HashMap<String, usize>> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        let stats = term.get_stats();
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
        Ok(result)
    }

    /// Count non-whitespace lines in visible screen
    ///
    /// Returns:
    ///     Number of lines containing non-whitespace characters
    fn count_non_whitespace_lines(&self) -> PyResult<usize> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.count_non_whitespace_lines())
    }

    /// Get scrollback usage
    ///
    /// Returns:
    ///     Tuple of (used_lines, max_capacity)
    fn get_scrollback_usage(&self) -> PyResult<(usize, usize)> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.get_scrollback_usage())
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
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.find_matching_bracket(col, row))
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
    fn select_semantic_region(
        &self,
        col: usize,
        row: usize,
        delimiters: &str,
    ) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.select_semantic_region(col, row, delimiters))
    }

    /// Export terminal content as HTML
    ///
    /// Args:
    ///     include_styles: Whether to include full HTML document with CSS (default: True)
    ///
    /// Returns:
    ///     HTML string with terminal content and styling
    #[pyo3(signature = (include_styles = true))]
    fn export_html(&self, include_styles: bool) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal.lock();
        Ok(term.export_html(include_styles))
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
        let terminal = self.inner.terminal();
        let limits = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.sixel_limits()
        } else {
            crate::sixel::SixelLimits::default()
        };
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
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_sixel_limits(max_width, max_height, max_repeat);
        }
        Ok(())
    }

    /// Get maximum number of Sixel graphics retained
    ///
    /// Returns:
    ///     Maximum number of in-memory Sixel graphics for this PTY terminal
    fn get_sixel_graphics_limit(&self) -> PyResult<usize> {
        let terminal = self.inner.terminal();
        let limit = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.max_sixel_graphics()
        } else {
            crate::sixel::SIXEL_DEFAULT_MAX_GRAPHICS
        };
        Ok(limit)
    }

    /// Set maximum number of Sixel graphics retained
    ///
    /// Args:
    ///     max_graphics: Maximum number of in-memory Sixel graphics
    ///
    /// Oldest graphics are dropped if the new limit is lower than the
    /// current number of graphics. The value is clamped to a safe range.
    fn set_sixel_graphics_limit(&mut self, max_graphics: usize) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = Ok::<_, ()>(terminal.lock()) {
            term.set_max_sixel_graphics(max_graphics);
        }
        Ok(())
    }

    /// Get count of Sixel graphics dropped due to limits
    ///
    /// Returns:
    ///     Number of Sixel graphics that have been dropped because of size or count limits
    fn get_dropped_sixel_graphics(&self) -> PyResult<usize> {
        let terminal = self.inner.terminal();
        let count = if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
            term.dropped_sixel_graphics()
        } else {
            0
        };
        Ok(count)
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
        let terminal = self.inner.terminal();
        let (limits, max_graphics, current_graphics, dropped_graphics) =
            if let Ok(term) = Ok::<_, ()>(terminal.lock()) {
                term.sixel_stats()
            } else {
                (
                    crate::sixel::SixelLimits::default(),
                    crate::sixel::SIXEL_DEFAULT_MAX_GRAPHICS,
                    0,
                    0,
                )
            };

        let mut stats = HashMap::new();
        stats.insert("max_width_px".to_string(), limits.max_width);
        stats.insert("max_height_px".to_string(), limits.max_height);
        stats.insert("max_repeat".to_string(), limits.max_repeat);
        stats.insert("max_graphics".to_string(), max_graphics);
        stats.insert("current_graphics".to_string(), current_graphics);
        stats.insert("dropped_graphics".to_string(), dropped_graphics);
        Ok(stats)
    }

    /// Start recording terminal session
    ///
    /// Args:
    ///     title: Optional session title (defaults to timestamp)
    fn start_recording(&self, title: Option<String>) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.start_recording(title);
        }
        Ok(())
    }

    /// Stop recording and return the session
    ///
    /// Returns:
    ///     RecordingSession object if recording was active, None otherwise
    fn stop_recording(&self) -> PyResult<Option<super::types::PyRecordingSession>> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term
                .stop_recording()
                .map(super::types::PyRecordingSession::from))
        } else {
            Ok(None)
        }
    }

    /// Check if terminal is currently recording
    ///
    /// Returns:
    ///     True if recording is active, False otherwise
    fn is_recording(&self) -> PyResult<bool> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.is_recording())
        } else {
            Ok(false)
        }
    }

    /// Export recording to asciicast v2 format
    ///
    /// Args:
    ///     session: RecordingSession from stop_recording()
    ///
    /// Returns:
    ///     Asciicast format string
    fn export_asciicast(
        &self,
        session: Option<&super::types::PyRecordingSession>,
        _py: Python,
    ) -> PyResult<String> {
        if let Some(session) = session {
            if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
                Ok(term.export_asciicast(&session.inner))
            } else {
                Err(PyRuntimeError::new_err("Failed to lock terminal"))
            }
        } else if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            if let Some(active) = term.get_recording_session() {
                Ok(term.export_asciicast(active))
            } else {
                Err(PyValueError::new_err(
                    "No active recording session (pass session=stop_recording())",
                ))
            }
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }

    /// Export recording to JSON format
    ///
    /// Returns:
    ///     JSON format string
    fn export_json(
        &self,
        session: Option<&super::types::PyRecordingSession>,
        _py: Python,
    ) -> PyResult<String> {
        if let Some(session) = session {
            if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
                Ok(term.export_json(&session.inner))
            } else {
                Err(PyRuntimeError::new_err("Failed to lock terminal"))
            }
        } else if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            if let Some(active) = term.get_recording_session() {
                Ok(term.export_json(active))
            } else {
                Err(PyValueError::new_err(
                    "No active recording session (pass session=stop_recording())",
                ))
            }
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }

    // === Macro Recording and Playback ===

    /// Load a macro into the library
    ///
    /// Args:
    ///     name: Name to store the macro under
    ///     macro: Macro object to load
    fn load_macro(&self, name: String, macro_obj: &super::types::PyMacro) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.load_macro(name, macro_obj.inner.clone());
        }
        Ok(())
    }

    /// Get a macro from the library
    ///
    /// Args:
    ///     name: Name of the macro to retrieve
    ///
    /// Returns:
    ///     Macro object if found, None otherwise
    fn get_macro(&self, name: String) -> PyResult<Option<super::types::PyMacro>> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term
                .get_macro(&name)
                .cloned()
                .map(super::types::PyMacro::from))
        } else {
            Ok(None)
        }
    }

    /// Remove a macro from the library
    ///
    /// Args:
    ///     name: Name of the macro to remove
    ///
    /// Returns:
    ///     Removed Macro object if found, None otherwise
    fn remove_macro(&self, name: String) -> PyResult<Option<super::types::PyMacro>> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.remove_macro(&name).map(super::types::PyMacro::from))
        } else {
            Ok(None)
        }
    }

    /// List all macro names
    ///
    /// Returns:
    ///     List of macro names
    fn list_macros(&self) -> PyResult<Vec<String>> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.list_macros())
        } else {
            Ok(Vec::new())
        }
    }

    /// Start playing a macro
    ///
    /// Args:
    ///     name: Name of the macro to play
    ///     speed: Playback speed multiplier (1.0 = normal, 2.0 = double speed)
    fn play_macro(&self, name: String, speed: Option<f64>) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.play_macro(&name, speed.unwrap_or(1.0))
                .map_err(PyValueError::new_err)
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }

    /// Stop macro playback
    fn stop_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.stop_macro();
        }
        Ok(())
    }

    /// Pause macro playback
    fn pause_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.pause_macro();
        }
        Ok(())
    }

    /// Resume macro playback
    fn resume_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.resume_macro();
        }
        Ok(())
    }

    /// Set macro playback speed
    ///
    /// Args:
    ///     speed: Speed multiplier (0.1 to 10.0)
    fn set_macro_speed(&self, speed: f64) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.set_macro_speed(speed);
        }
        Ok(())
    }

    /// Check if a macro is currently playing
    ///
    /// Returns:
    ///     True if a macro is playing, False otherwise
    fn is_macro_playing(&self) -> PyResult<bool> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.is_macro_playing())
        } else {
            Ok(false)
        }
    }

    /// Check if macro playback is paused
    ///
    /// Returns:
    ///     True if paused, False otherwise
    fn is_macro_paused(&self) -> PyResult<bool> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.is_macro_paused())
        } else {
            Ok(false)
        }
    }

    /// Get macro playback progress
    ///
    /// Returns:
    ///     Tuple of (current_event, total_events) if playing, None otherwise
    fn get_macro_progress(&self) -> PyResult<Option<(usize, usize)>> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.get_macro_progress())
        } else {
            Ok(None)
        }
    }

    /// Get the name of the currently playing macro
    ///
    /// Returns:
    ///     Macro name if playing, None otherwise
    fn get_current_macro_name(&self) -> PyResult<Option<String>> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.get_current_macro_name())
        } else {
            Ok(None)
        }
    }

    /// Tick macro playback and send events to PTY
    ///
    /// Call this regularly (e.g., every 10ms) to advance macro playback
    ///
    /// Returns:
    ///     True if an event was processed, False otherwise
    fn tick_macro(&mut self) -> PyResult<bool> {
        let bytes = if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            term.tick_macro()
        } else {
            None
        };

        if let Some(bytes) = bytes {
            self.write(&bytes)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get and clear screenshot triggers from macro playback
    ///
    /// Returns:
    ///     List of screenshot labels
    fn get_macro_screenshot_triggers(&self) -> PyResult<Vec<String>> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(term.get_macro_screenshot_triggers())
        } else {
            Ok(Vec::new())
        }
    }

    /// Convert a recording session to a macro
    ///
    /// Args:
    ///     session: RecordingSession to convert
    ///     name: Name for the new macro
    ///
    /// Returns:
    ///     Macro object
    fn recording_to_macro(
        &self,
        session: &super::types::PyRecordingSession,
        name: String,
    ) -> PyResult<super::types::PyMacro> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().lock()) {
            Ok(super::types::PyMacro::from(
                term.recording_to_macro(&session.inner, name),
            ))
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }
}

// Rust-only methods (not exposed to Python)
#[allow(dead_code)] // Used by streaming feature
impl PyPtyTerminal {
    /// Get a clone of the terminal Arc (for use in streaming server)
    pub(crate) fn get_terminal_arc(
        &self,
    ) -> std::sync::Arc<parking_lot::Mutex<crate::terminal::Terminal>> {
        self.inner.terminal()
    }

    /// Set an output callback on the PtySession
    ///
    /// This is used internally to wire up streaming servers
    pub(crate) fn set_output_callback(&mut self, callback: crate::pty_session::OutputCallback) {
        self.inner.set_output_callback(callback);
    }

    /// Get the PTY writer for streaming server input handling
    ///
    /// Returns a thread-safe writer that can be used to send input to the PTY
    pub(crate) fn get_pty_writer(
        &self,
    ) -> Option<std::sync::Arc<parking_lot::Mutex<Box<dyn std::io::Write + Send>>>> {
        self.inner.get_writer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tests for PtySession wrapper (testing the underlying Rust behavior)
    // Note: These tests test through the inner PtySession since PyO3 types
    // require Python interpreter setup for full testing.
    // =========================================================================

    // -------------------------------------------------------------------------
    // Creation and initialization tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_pty_session_creation() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        assert_eq!(session.size(), (80, 24));
        assert!(!session.is_running());
    }

    #[test]
    fn test_pty_session_creation_different_sizes() {
        let session1 = pty_session::PtySession::new(40, 20, 500);
        assert_eq!(session1.size(), (40, 20));

        let session2 = pty_session::PtySession::new(200, 60, 5000);
        assert_eq!(session2.size(), (200, 60));
    }

    #[test]
    fn test_pty_session_initial_state() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        assert!(!session.is_running());
        assert_eq!(session.update_generation(), 0);
        assert_eq!(session.cursor_position(), (0, 0));
        assert!(session.content().is_empty() || session.content().trim().is_empty());
    }

    // -------------------------------------------------------------------------
    // Environment variable tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_env_basic() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_env("TEST_VAR", "test_value");
        // Should not panic
    }

    #[test]
    fn test_set_env_multiple() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_env("VAR1", "value1");
        session.set_env("VAR2", "value2");
        session.set_env("VAR3", "value3");
        // Should handle multiple env vars
    }

    #[test]
    fn test_set_env_empty_value() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_env("EMPTY_VAR", "");
        // Should handle empty values
    }

    #[test]
    fn test_set_env_unicode() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_env("UNICODE_VAR", "Hello 世界 🌍");
        // Should handle unicode
    }

    #[test]
    fn test_set_env_special_chars() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_env("PATH_VAR", "/usr/bin:/usr/local/bin");
        session.set_env("QUOTE_VAR", "value with \"quotes\"");
        // Should handle special characters
    }

    // -------------------------------------------------------------------------
    // Working directory tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_cwd() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let path = std::path::Path::new("/tmp");
        session.set_cwd(path);
        // Should not panic
    }

    #[test]
    fn test_set_cwd_home() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(home) = std::env::var("HOME") {
            session.set_cwd(std::path::Path::new(&home));
        }
    }

    // -------------------------------------------------------------------------
    // Resize tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_resize() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.resize(100, 30).ok();
        assert_eq!(session.size(), (100, 30));
    }

    #[test]
    fn test_resize_multiple() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);

        session.resize(100, 30).ok();
        assert_eq!(session.size(), (100, 30));

        session.resize(120, 40).ok();
        assert_eq!(session.size(), (120, 40));

        session.resize(60, 20).ok();
        assert_eq!(session.size(), (60, 20));
    }

    #[test]
    fn test_resize_small() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.resize(10, 5).ok();
        assert_eq!(session.size(), (10, 5));
    }

    #[test]
    fn test_resize_large() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.resize(500, 200).ok();
        assert_eq!(session.size(), (500, 200));
    }

    #[test]
    fn test_resize_with_pixels() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.resize_with_pixels(100, 30, 1000, 600).ok();
        assert_eq!(session.size(), (100, 30));
    }

    // -------------------------------------------------------------------------
    // Terminal access tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_terminal_access() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let terminal = session.terminal();
        let _guard = terminal.lock();
    }

    #[test]
    fn test_terminal_content_empty() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let content = session.content();
        assert!(content.is_empty() || content.chars().all(|c| c.is_whitespace()));
    }

    #[test]
    fn test_terminal_process_direct() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            term.process(b"Hello, World!");
            let content = term.content();
            assert!(content.contains("Hello, World!"));
        }
    }

    // -------------------------------------------------------------------------
    // Update generation tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_update_generation_initial() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        assert_eq!(session.update_generation(), 0);
    }

    #[test]
    fn test_update_generation_stable() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let gen1 = session.update_generation();
        let gen2 = session.update_generation();
        assert_eq!(gen1, gen2);
    }

    #[test]
    fn test_has_updates_since() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let gen = session.update_generation();
        assert!(!session.has_updates_since(gen));
    }

    // -------------------------------------------------------------------------
    // Bell count tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_bell_count_initial() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        assert_eq!(session.bell_count(), 0);
    }

    #[test]
    fn test_bell_count_after_bell() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            term.process(b"\x07"); // BEL character
        }
        assert_eq!(session.bell_count(), 1);
    }

    // -------------------------------------------------------------------------
    // Scrollback tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_scrollback_empty() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        assert_eq!(session.scrollback_len(), 0);
    }

    #[test]
    fn test_scrollback_content_empty() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let scrollback = session.scrollback();
        assert!(scrollback.is_empty());
    }

    // -------------------------------------------------------------------------
    // Get line tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_line_valid() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            term.process(b"Line0\nLine1\nLine2");
        }
        let line = session.get_line(0);
        assert!(line.is_some());
    }

    #[test]
    fn test_get_line_out_of_bounds() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let line = session.get_line(100);
        assert!(line.is_none());
    }

    // -------------------------------------------------------------------------
    // Export tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_export_text_empty() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let text = session.export_text();
        // Empty terminal might have whitespace or be empty
        assert!(text.chars().all(|c| c.is_whitespace()) || text.is_empty());
    }

    #[test]
    fn test_export_text_with_content() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            term.process(b"Test content here");
        }
        let text = session.export_text();
        assert!(text.contains("Test content here"));
    }

    #[test]
    fn test_export_styled_with_content() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            // Add colored text
            term.process(b"\x1b[31mRed text\x1b[0m");
        }
        let styled = session.export_styled();
        assert!(styled.contains("Red text"));
    }

    // -------------------------------------------------------------------------
    // Write without running tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_write_without_running() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let result = session.write(b"test");
        assert!(result.is_err());
    }

    #[test]
    fn test_write_str_without_running() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let result = session.write_str("test");
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Kill without running tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_kill_without_running() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let result = session.kill();
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Wait without running tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_wait_without_running() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let result = session.wait();
        assert!(result.is_err());
    }

    #[test]
    fn test_try_wait_without_running() {
        let mut session = pty_session::PtySession::new(80, 24, 1000);
        let result = session.try_wait();
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Default shell tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_default_shell() {
        let shell = pty_session::PtySession::get_default_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    fn test_get_default_shell_valid() {
        let shell = pty_session::PtySession::get_default_shell();
        #[cfg(unix)]
        assert!(
            shell.contains("sh") || shell.contains("zsh") || shell.contains("fish"),
            "Shell should be a known shell: {}",
            shell
        );
    }

    // -------------------------------------------------------------------------
    // Output callback tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_set_output_callback() {
        use std::sync::Arc;

        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_output_callback(Arc::new(|_data| {
            // Just verify callback can be set
        }));
        // Should not panic
    }

    #[test]
    fn test_clear_output_callback() {
        use std::sync::Arc;

        let mut session = pty_session::PtySession::new(80, 24, 1000);
        session.set_output_callback(Arc::new(|_data| {}));
        session.clear_output_callback();
        // Should not panic
    }

    // -------------------------------------------------------------------------
    // Writer access tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_writer_without_running() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let writer = session.get_writer();
        assert!(writer.is_none());
    }

    // -------------------------------------------------------------------------
    // Cursor position tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_cursor_position_initial() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        let (col, row) = session.cursor_position();
        assert_eq!(col, 0);
        assert_eq!(row, 0);
    }

    #[test]
    fn test_cursor_position_after_write() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            term.process(b"Hello");
            let cursor = term.cursor();
            assert_eq!(cursor.col, 5);
            assert_eq!(cursor.row, 0);
        }
    }

    // -------------------------------------------------------------------------
    // Terminal state tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_terminal_process_escape_sequences() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            // Test cursor movement
            term.process(b"\x1b[5;10H"); // Move to row 5, col 10
            let cursor = term.cursor();
            assert_eq!(cursor.row, 4); // 0-indexed
            assert_eq!(cursor.col, 9); // 0-indexed
        }
    }

    #[test]
    fn test_terminal_alt_screen() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            assert!(!term.is_alt_screen_active());

            // Enter alt screen
            term.process(b"\x1b[?1049h");
            assert!(term.is_alt_screen_active());

            // Exit alt screen
            term.process(b"\x1b[?1049l");
            assert!(!term.is_alt_screen_active());
        }
    }

    #[test]
    fn test_terminal_colors() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().lock()) {
            // Set red foreground
            term.process(b"\x1b[31mRed\x1b[0m");
            let cell = term.active_grid().get(0, 0);
            assert!(cell.is_some());
            if let Some(cell) = cell {
                assert_eq!(cell.c, 'R');
            }
        }
    }

    // =========================================================================
    // Helper function tests (conversions module)
    // =========================================================================

    #[test]
    fn test_parse_sixel_mode_disabled() {
        let result = super::super::conversions::parse_sixel_mode("disabled");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_sixel_mode_pixels() {
        let result = super::super::conversions::parse_sixel_mode("pixels");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_sixel_mode_halfblocks() {
        let result = super::super::conversions::parse_sixel_mode("halfblocks");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_sixel_mode_invalid() {
        let result = super::super::conversions::parse_sixel_mode("invalid_mode");
        assert!(result.is_err());
    }
}
