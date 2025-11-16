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
    /// On Unix: Uses $SHELL or defaults to /bin/sh
    /// On Windows: Uses PowerShell or cmd.exe
    fn spawn_shell(&mut self) -> PyResult<()> {
        self.inner.spawn_shell()?;
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
        let title = if let Ok(term) = terminal.lock() {
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
        use_bold_color = None
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
            sixel_render_mode: parse_sixel_mode(sixel_mode)?,
            link_color,
            bold_color,
            use_bold_color: use_bold_color.unwrap_or(false),
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
        use_bold_color = None
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
    ) -> PyResult<Option<Vec<(char, (u8, u8, u8), (u8, u8, u8), PyAttributes)>>> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        let grid = term.grid();
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let result = if let Ok(term) = terminal.lock() {
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;

        // Get current grid (will be either primary or alternate)
        let grid = term.active_grid();
        let rows = grid.rows();
        let cols = grid.cols();

        // Capture all lines while holding terminal lock
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

    /// Get mouse tracking mode
    ///
    /// Returns:
    ///     String representing the mouse mode: "off", "x10", "normal", "button", "any"
    fn mouse_mode(&self) -> PyResult<String> {
        use crate::mouse::MouseMode;
        let terminal = self.inner.terminal();
        let mode = if let Ok(term) = terminal.lock() {
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
        let visible = if let Ok(term) = terminal.lock() {
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
        let flags = if let Ok(term) = terminal.lock() {
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
        let mode = if let Ok(term) = terminal.lock() {
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
        let mode = if let Ok(term) = terminal.lock() {
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

    /// Get clipboard content (OSC 52)
    ///
    /// Returns:
    ///     Clipboard content as string, or None if empty
    fn clipboard(&self) -> PyResult<Option<String>> {
        let terminal = self.inner.terminal();
        let content = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let allowed = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let color = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let color = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let color = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
            term.set_use_bold_color(use_bold);
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
        if let Ok(mut term) = terminal.lock() {
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
        let style = if let Ok(term) = terminal.lock() {
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
        let mut term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        term.process(sequence.as_bytes());
        Ok(())
    }

    /// Check if alternate screen is active
    ///
    /// Returns:
    ///     True if alternate screen is active
    fn is_alt_screen_active(&self) -> PyResult<bool> {
        let terminal = self.inner.terminal();
        let is_alt = if let Ok(term) = terminal.lock() {
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
        let enabled = if let Ok(term) = terminal.lock() {
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
        let event = if let Ok(term) = terminal.lock() {
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
        let event = if let Ok(term) = terminal.lock() {
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
        let enabled = if let Ok(term) = terminal.lock() {
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
        let sequence = if let Ok(term) = terminal.lock() {
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
        let sequence = if let Ok(term) = terminal.lock() {
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
        if let Ok(term) = terminal.lock() {
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
        let enabled = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let responses = if let Ok(mut term) = terminal.lock() {
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
        let has_pending = if let Ok(term) = terminal.lock() {
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
        let has_pending = if let Ok(term) = terminal.lock() {
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
        let notifications = if let Ok(mut term) = terminal.lock() {
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

    /// Get a debug snapshot of the current buffer state
    ///
    /// Returns:
    ///     String containing a formatted view of the buffer
    fn debug_snapshot_buffer(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        let grid = term.active_grid();
        Ok(grid.debug_snapshot())
    }

    /// Get a debug snapshot of the grid
    ///
    /// Returns:
    ///     String containing a formatted view of the grid
    fn debug_snapshot_grid(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        Ok(term.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the primary screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the primary buffer
    fn debug_snapshot_primary(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        Ok(term.grid().debug_snapshot())
    }

    /// Get a debug snapshot of the alternate screen buffer
    ///
    /// Returns:
    ///     String containing a formatted view of the alternate buffer
    fn debug_snapshot_alt(&self) -> PyResult<String> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        Ok(term.alt_grid().debug_snapshot())
    }

    /// Log a debug snapshot with a label
    ///
    /// Args:
    ///     label: Description of this snapshot
    fn debug_log_snapshot(&self, label: &str) -> PyResult<()> {
        use crate::debug;
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let cwd = if let Ok(term) = terminal.lock() {
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
        let accepted = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let disabled = if let Ok(term) = terminal.lock() {
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
        if let Ok(mut term) = terminal.lock() {
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;

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
        let graphics = if let Ok(term) = terminal.lock() {
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
        let count = if let Ok(term) = terminal.lock() {
            term.graphics_count()
        } else {
            0
        };
        Ok(count)
    }

    /// Clear all graphics
    fn clear_graphics(&mut self) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(mut term) = terminal.lock() {
            term.clear_graphics();
        }
        Ok(())
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
    ///     word_chars: Optional custom word characters (default: "-_.~:/?#[]@!$&'()*+,;=")
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
        Ok(term.count_non_whitespace_lines())
    }

    /// Get scrollback usage
    ///
    /// Returns:
    ///     Tuple of (used_lines, max_capacity)
    fn get_scrollback_usage(&self) -> PyResult<(usize, usize)> {
        let terminal = self.inner.terminal();
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
        let term = terminal
            .lock()
            .map_err(|e| PyRuntimeError::new_err(format!("Lock error: {}", e)))?;
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
}
