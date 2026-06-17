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
use super::types::{PyAttributes, PyScreenSnapshot};

/// Python wrapper for PtySession - a terminal with PTY support
#[pyclass(name = "PtyTerminal", unsendable)]
pub struct PyPtyTerminal {
    inner: pty_session::PtySession,
}

// ARC-003/QA-001: unified Terminal access so shared methods can be emitted once
// (see `python_bindings::common`).
impl crate::python_bindings::common::TerminalAccess for PyPtyTerminal {
    fn term_ref(&self) -> impl std::ops::Deref<Target = crate::terminal::Terminal> {
        // Shared read access — multiple Python read queries can proceed
        // concurrently (ARC-009). Methods using term_ref are &self-only
        // (Deref, not DerefMut), so the compiler guarantees they don't mutate.
        self.inner.terminal_ref().read()
    }
    fn term_mut(&mut self) -> impl std::ops::DerefMut<Target = crate::terminal::Terminal> {
        self.inner.terminal_ref().write()
    }
}

// ARC-003/QA-001 validation: shared getters generated from one definition.
crate::impl_terminal_simple_getters!(PyPtyTerminal);
crate::impl_terminal_query_getters!(PyPtyTerminal);
crate::impl_terminal_color_setters!(PyPtyTerminal);
crate::impl_terminal_state_setters!(PyPtyTerminal);
crate::impl_terminal_static_helpers!(PyPtyTerminal);
crate::impl_terminal_sixel_graphics!(PyPtyTerminal);
crate::impl_terminal_badge_session!(PyPtyTerminal);
crate::impl_terminal_progress_notifications!(PyPtyTerminal);
crate::impl_terminal_recording!(PyPtyTerminal);
crate::impl_terminal_cell_line_queries!(PyPtyTerminal);
crate::impl_terminal_content_misc!(PyPtyTerminal);
crate::impl_terminal_search_select!(PyPtyTerminal);
crate::impl_terminal_debug_snapshots!(PyPtyTerminal);
crate::impl_terminal_file_transfer!(PyPtyTerminal);
crate::impl_terminal_exports!(PyPtyTerminal);

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

    /// Return the PID of the spawned child process.
    ///
    /// Returns:
    ///     PID as an integer, or None if no process has been spawned yet.
    ///
    /// Example:
    ///     >>> session = PtySession(80, 24)
    ///     >>> session.spawn_shell()
    ///     >>> pid = session.child_pid()
    ///     >>> print(pid)  # e.g. 12345
    fn child_pid(&self) -> PyResult<Option<u32>> {
        Ok(self.inner.child_pid())
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

    // content, __str__: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // title: provided by impl_terminal_query_getters! (ARC-003/QA-001)
    // export_text, export_styled: provided by impl_terminal_exports! (ARC-003/QA-001)

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
        let (term_bold_brightening, term_bg_color) = if let Ok(term) = Ok::<_, ()>(terminal.write())
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

    /// Take a screenshot using a reusable [`PyScreenshotConfig`] (QA-005).
    ///
    /// Build a config once and pass it, instead of repeating 16+ keyword args:
    /// ```python
    /// cfg = ScreenshotConfig(format="png", font_size=16.0, render_cursor=True)
    /// term.screenshot_config(cfg, scrollback_offset=0)
    /// ```
    #[pyo3(signature = (config, scrollback_offset=0))]
    fn screenshot_config(
        &self,
        config: &super::screenshot_config::PyScreenshotConfig,
        scrollback_offset: usize,
    ) -> PyResult<Vec<u8>> {
        let cfg = config.to_screenshot_config()?;
        self.inner
            .screenshot(cfg, scrollback_offset)
            .map_err(|e| PyRuntimeError::new_err(format!("Screenshot error: {}", e)))
    }

    /// Take a screenshot to a file using a reusable [`PyScreenshotConfig`] (QA-005).
    #[pyo3(signature = (path, config, scrollback_offset=0))]
    fn screenshot_to_file_config(
        &self,
        path: &str,
        config: &super::screenshot_config::PyScreenshotConfig,
        scrollback_offset: usize,
    ) -> PyResult<()> {
        let bytes = self.screenshot_config(config, scrollback_offset)?;
        std::fs::write(path, bytes)
            .map_err(|e| PyIOError::new_err(format!("Failed to write file: {}", e)))
    }

    // size: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // cursor_position: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // scrollback: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Get the number of scrollback lines
    ///
    /// Returns:
    ///     Number of lines in scrollback buffer
    fn scrollback_len(&self) -> PyResult<usize> {
        Ok(self.inner.scrollback_len())
    }

    // scrollback_line: provided by impl_terminal_search_select! (ARC-003/QA-001)

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
        let result = if let Ok(term) = Ok::<_, ()>(terminal.write()) {
            term.active_grid().get(col, row).map(|cell| cell.c)
        } else {
            None
        };
        Ok(result)
    }

    // is_line_wrapped, get_fg_color, get_bg_color, get_underline_color, get_attributes,
    // get_hyperlink, get_line_cells: provided by impl_terminal_cell_line_queries! (ARC-003/QA-001)

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
        let term = terminal.write();

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
                        PyAttributes::from(cell),
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

    // === Coprocess Management ===

    /// Start a new coprocess
    ///
    /// The coprocess receives terminal output on its stdin (if copy_terminal_output
    /// is True) and its stdout is buffered for reading via read_from_coprocess().
    ///
    /// Args:
    ///     config: CoprocessConfig with command and options
    ///
    /// Returns:
    ///     int: Coprocess ID for future reference
    ///
    /// Example:
    ///     >>> config = CoprocessConfig("grep", args=["ERROR"])
    ///     >>> coproc_id = pty.start_coprocess(config)
    fn start_coprocess(&self, config: super::types::PyCoprocessConfig) -> PyResult<u64> {
        let rust_config = crate::coprocess::CoprocessConfig::from(&config);
        self.inner
            .start_coprocess(rust_config)
            .map_err(PyRuntimeError::new_err)
    }

    /// Stop a coprocess by ID
    ///
    /// Args:
    ///     coprocess_id: ID of the coprocess to stop
    fn stop_coprocess(&self, coprocess_id: u64) -> PyResult<()> {
        self.inner
            .stop_coprocess(coprocess_id)
            .map_err(PyRuntimeError::new_err)
    }

    /// Write data to a coprocess's stdin
    ///
    /// Args:
    ///     coprocess_id: ID of the coprocess
    ///     data: Bytes to write
    fn write_to_coprocess(&self, coprocess_id: u64, data: &[u8]) -> PyResult<()> {
        self.inner
            .write_to_coprocess(coprocess_id, data)
            .map_err(PyRuntimeError::new_err)
    }

    /// Read buffered output from a coprocess (drains the buffer)
    ///
    /// Args:
    ///     coprocess_id: ID of the coprocess
    ///
    /// Returns:
    ///     list[str]: Lines of output from the coprocess
    fn read_from_coprocess(&self, coprocess_id: u64) -> PyResult<Vec<String>> {
        self.inner
            .read_from_coprocess(coprocess_id)
            .map_err(PyRuntimeError::new_err)
    }

    /// List all coprocess IDs
    ///
    /// Returns:
    ///     list[int]: List of active coprocess IDs
    fn list_coprocesses(&self) -> PyResult<Vec<u64>> {
        Ok(self.inner.list_coprocesses())
    }

    /// Check if a coprocess is still running
    ///
    /// Args:
    ///     coprocess_id: ID of the coprocess
    ///
    /// Returns:
    ///     bool | None: True if running, False if exited, None if not found
    fn coprocess_status(&self, coprocess_id: u64) -> PyResult<Option<bool>> {
        Ok(self.inner.coprocess_status(coprocess_id))
    }

    /// Read buffered stderr output from a coprocess (drains the buffer)
    ///
    /// Args:
    ///     coprocess_id: ID of the coprocess
    ///
    /// Returns:
    ///     list[str]: Lines of stderr output from the coprocess
    ///
    /// Example:
    ///     >>> errors = pty.read_coprocess_errors(coproc_id)
    ///     >>> for line in errors:
    ///     ...     print(f"ERROR: {line}")
    fn read_coprocess_errors(&self, coprocess_id: u64) -> PyResult<Vec<String>> {
        self.inner
            .read_coprocess_errors(coprocess_id)
            .map_err(PyRuntimeError::new_err)
    }

    // mouse_mode: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // cursor_visible: provided by impl_terminal_simple_getters! (ARC-003/QA-001)

    // keyboard_flags: provided by impl_terminal_query_getters! (ARC-003/QA-001)

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

    // insert_mode: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // line_feed_new_line_mode: provided by impl_terminal_query_getters! (ARC-003/QA-001)

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
        let mut term = terminal.write();
        term.set_keyboard_flags(flags);
        Ok(())
    }

    // clipboard: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // set_clipboard: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // allow_clipboard_read: provided by impl_terminal_simple_getters! (ARC-003/QA-001)

    // set_allow_clipboard_read: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // default_fg: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_default_fg: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    /// Query default foreground color (OSC 10)
    ///
    /// Sends OSC 10 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 10 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_fg(&mut self) -> PyResult<()> {
        self.write(b"\x1b]10;?\x1b\\")?;
        Ok(())
    }

    // default_bg: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_default_bg: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    /// Query default background color (OSC 11)
    ///
    /// Sends OSC 11 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 11 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_bg(&mut self) -> PyResult<()> {
        self.write(b"\x1b]11;?\x1b\\")?;
        Ok(())
    }

    // Get cursor color: provided by impl_terminal_simple_getters! (ARC-003/QA-001)

    // set_cursor_color: provided by impl_terminal_state_setters! (ARC-003/QA-001)

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
        if let Ok(mut term) = Ok::<_, ()>(terminal.write()) {
            term.set_ansi_palette_color(index, Color::Rgb(r, g, b))
                .map_err(PyErr::new::<pyo3::exceptions::PyValueError, _>)?;
        }
        Ok(())
    }

    // set_link_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_bold_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_cursor_guide_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_badge_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_match_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_selection_bg_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_selection_fg_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_use_bold_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_bold_brightening: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // faint_text_alpha: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_faint_text_alpha: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // set_use_underline_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // cursor_style: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // set_cursor_style: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // is_alt_screen_active: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // focus_tracking: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // get_focus_in_event, get_focus_out_event: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // bracketed_paste: provided by impl_terminal_simple_getters! (ARC-003/QA-001)

    // get_paste_start, get_paste_end: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    /// Paste text content into terminal with bracketed paste support
    ///
    /// If bracketed paste mode is enabled, wraps the content with ESC[200~ and ESC[201~
    /// Otherwise, writes the content directly to the PTY
    ///
    /// Args:
    ///     content: String content to paste
    fn paste(&mut self, content: &str) -> PyResult<()> {
        let terminal = self.inner.terminal();
        if let Ok(term) = Ok::<_, ()>(terminal.write()) {
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

    // synchronized_updates: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // flush_synchronized_updates: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // Device query response methods
    // drain_responses, has_pending_responses, has_notifications, take_notifications,
    // drain_notifications, progress_bar, has_progress, progress_value, progress_state,
    // set_progress, clear_progress: provided by impl_terminal_progress_notifications! (ARC-003/QA-001)

    // debug_snapshot_buffer, debug_snapshot_grid, debug_snapshot_primary,
    // debug_snapshot_alt, debug_log_snapshot:
    //   provided by impl_terminal_debug_snapshots! (ARC-003/QA-001)

    // shell_integration_state: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // current_directory: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // accept_osc7: provided by impl_terminal_simple_getters! (ARC-003/QA-001)

    // set_accept_osc7: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // answerback_string: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_answerback_string: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // width_config: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_width_config: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // set_ambiguous_width: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // set_unicode_version: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // char_width: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // disable_insecure_sequences, set_disable_insecure_sequences:
    //   provided by impl_terminal_content_misc! (ARC-003/QA-001)

    /// Get current debug information as a dictionary
    ///
    /// Returns:
    ///     Dictionary containing terminal state for debugging
    fn debug_info(&self) -> PyResult<HashMap<String, String>> {
        let terminal = self.inner.terminal();
        let term = terminal.write();

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
    // graphics_at_row, graphics_count, graphics, clear_graphics:
    //   provided by impl_terminal_sixel_graphics! (ARC-003/QA-001)
    // update_animations: provided by impl_terminal_exports! (ARC-003/QA-001)

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

    // __str__: provided by impl_terminal_content_misc! (ARC-003/QA-001)

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
    // get_word_at, get_url_at, get_line_unwrapped:
    //   provided by impl_terminal_cell_line_queries! (ARC-003/QA-001)

    // select_word, find_text, find_next:
    //   provided by impl_terminal_search_select! (ARC-003/QA-001)

    // ========== Content Search ==========

    // ========== Buffer Statistics ==========

    /// Get terminal statistics
    ///
    /// Returns:
    ///     Dictionary with statistics: cols, rows, scrollback_lines, total_cells,
    ///     non_whitespace_lines, graphics_count, estimated_memory_bytes
    fn get_stats(&self) -> PyResult<HashMap<String, usize>> {
        let terminal = self.inner.terminal();
        let term = terminal.write();
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

    // count_non_whitespace_lines: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Get scrollback usage
    ///
    /// Returns:
    ///     Tuple of (used_lines, max_capacity)
    fn get_scrollback_usage(&self) -> PyResult<(usize, usize)> {
        let terminal = self.inner.terminal();
        let term = terminal.write();
        Ok((term.get_scrollback_usage(), term.grid().max_scrollback()))
    }

    // find_matching_bracket, select_semantic_region:
    //   provided by impl_terminal_search_select! (ARC-003/QA-001)
    // export_html: provided by impl_terminal_exports! (ARC-003/QA-001)

    // ========== Static Utility Methods ==========
    // strip_ansi, measure_text_width, parse_color: provided by impl_terminal_static_helpers! (ARC-003/QA-001)
    // get_sixel_limits, set_sixel_limits, get_sixel_graphics_limit, set_sixel_graphics_limit,
    // get_dropped_sixel_graphics, get_sixel_stats: provided by impl_terminal_sixel_graphics! (ARC-003/QA-001)

    // start_recording, stop_recording, is_recording, record_output, record_input,
    // record_resize, record_marker, get_recording_session:
    //   provided by impl_terminal_recording! (ARC-003/QA-001)

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
            if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
                Ok(term.export_asciicast(&session.inner))
            } else {
                Err(PyRuntimeError::new_err("Failed to lock terminal"))
            }
        } else if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
            if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
                Ok(term.export_json(&session.inner))
            } else {
                Err(PyRuntimeError::new_err("Failed to lock terminal"))
            }
        } else if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
    #[pyo3(signature = (name, speed=None))]
    fn play_macro(&self, name: String, speed: Option<f64>) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
            term.play_macro(&name).map_err(PyValueError::new_err)?;
            if let Some(s) = speed {
                term.set_macro_speed(s);
            }
            Ok(())
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }

    /// Stop macro playback
    fn stop_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
            term.stop_macro();
        }
        Ok(())
    }

    /// Pause macro playback
    fn pause_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
            term.pause_macro();
        }
        Ok(())
    }

    /// Resume macro playback
    fn resume_macro(&self) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
            term.resume_macro();
        }
        Ok(())
    }

    /// Set macro playback speed
    ///
    /// Args:
    ///     speed: Speed multiplier (0.1 to 10.0)
    fn set_macro_speed(&self, speed: f64) -> PyResult<()> {
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
            term.set_macro_speed(speed);
        }
        Ok(())
    }

    /// Check if a macro is currently playing
    ///
    /// Returns:
    ///     True if a macro is playing, False otherwise
    fn is_macro_playing(&self) -> PyResult<bool> {
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        let bytes = if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(self.inner.terminal().write()) {
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
        if let Ok(term) = Ok::<_, ()>(self.inner.terminal().write()) {
            Ok(super::types::PyMacro::from(
                term.recording_to_macro(&session.inner, name),
            ))
        } else {
            Err(PyRuntimeError::new_err("Failed to lock terminal"))
        }
    }

    // ========== Badge Format Support (OSC 1337 SetBadgeFormat) ==========
    // badge_format, set_badge_format, clear_badge_format, evaluate_badge,
    // get_badge_session_variable, set_badge_session_variable, get_badge_session_variables:
    //   provided by impl_terminal_badge_session! (ARC-003/QA-001)

    // =========================================================================
    // File Transfer API
    // =========================================================================
    // get_active_transfers, get_completed_transfers, get_transfer,
    // take_completed_transfer, cancel_file_transfer, send_upload_data,
    // cancel_upload, set_max_transfer_size, get_max_transfer_size:
    //   provided by impl_terminal_file_transfer! (ARC-003/QA-001)
}

// Rust-only methods (not exposed to Python)
#[cfg(feature = "streaming")]
impl PyPtyTerminal {
    /// Get a clone of the terminal Arc (for use in streaming server)
    pub(crate) fn get_terminal_arc(
        &self,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::terminal::Terminal>> {
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
        let _guard = terminal.write();
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
            term.process(b"Test content here");
        }
        let text = session.export_text();
        assert!(text.contains("Test content here"));
    }

    #[test]
    fn test_export_styled_with_content() {
        let session = pty_session::PtySession::new(80, 24, 1000);
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
        if let Ok(mut term) = Ok::<_, ()>(session.terminal().write()) {
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
