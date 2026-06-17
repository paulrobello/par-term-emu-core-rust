//! Python bindings for the Terminal emulator
//!
//! This module contains the `PyTerminal` struct and its implementation,
//! providing the main Python interface for terminal emulation functionality.

// ARC-002: cohesive method groups are split into sibling `*_api` files, each
// with its own `#[pymethods] impl PyTerminal` block. Pure relocation — the
// Python `Terminal` class keeps the same surface.
mod badge_api;
mod bookmark_api;
mod clipboard_api;
mod color_api;
mod file_transfer_api;
mod image_api;
mod metrics_api;
mod mouse_api;
mod multiplexing_api;
mod notification_api;
mod recording_api;
mod scrollback_api;
mod search_api;
mod selection_api;
mod shell_integration_api;
mod text_api;
mod trigger_api;

use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::color::Color;

use super::enums::PyMouseEncoding;
use super::types::{PyAttributes, PyScreenSnapshot};

/// Python wrapper for the Terminal
#[pyclass(name = "Terminal")]
pub struct PyTerminal {
    pub(crate) inner: crate::terminal::Terminal,
}

// ARC-003/QA-001: unified Terminal access so shared methods can be emitted once
// (see `python_bindings::common`).
impl crate::python_bindings::common::TerminalAccess for PyTerminal {
    fn term_ref(&self) -> impl std::ops::Deref<Target = crate::terminal::Terminal> {
        &self.inner
    }
    fn term_mut(&mut self) -> impl std::ops::DerefMut<Target = crate::terminal::Terminal> {
        &mut self.inner
    }
}

// ARC-003/QA-001: shared query/state getters generated from one definition.
crate::impl_terminal_query_getters!(PyTerminal);
crate::impl_terminal_color_setters!(PyTerminal);
crate::impl_terminal_state_setters!(PyTerminal);
crate::impl_terminal_static_helpers!(PyTerminal);
crate::impl_terminal_sixel_graphics!(PyTerminal);
crate::impl_terminal_badge_session!(PyTerminal);
crate::impl_terminal_progress_notifications!(PyTerminal);
crate::impl_terminal_recording!(PyTerminal);
crate::impl_terminal_cell_line_queries!(PyTerminal);
crate::impl_terminal_content_misc!(PyTerminal);
crate::impl_terminal_search_select!(PyTerminal);
crate::impl_terminal_debug_snapshots!(PyTerminal);
crate::impl_terminal_file_transfer!(PyTerminal);
crate::impl_terminal_exports!(PyTerminal);

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

    // content, __str__: provided by impl_terminal_content_misc! (ARC-003/QA-001)
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
            bold_brightening: bold_brightening.unwrap_or(false),
            background_color,
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
    /// Avoids repeating 16+ keyword args on every call. Build a config once
    /// and pass it here:
    /// ```python
    /// cfg = ScreenshotConfig(format="png", font_size=16.0, render_cursor=True)
    /// term.screenshot_config(cfg, scrollback_offset=0)
    /// ```
    ///
    /// Args:
    ///     config: A `ScreenshotConfig` (keyword-arg constructor).
    ///     scrollback_offset: Lines to scroll back from current position.
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

    // title: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Set the terminal title directly
    ///
    /// This sets the title without using OSC sequences.
    /// Useful for programmatic control.
    ///
    /// Args:
    ///     title: The new title string
    fn set_title(&mut self, title: String) -> PyResult<()> {
        self.inner.set_title(title);
        Ok(())
    }

    // cursor_position: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Check if cursor is visible
    ///
    /// Returns:
    ///     True if cursor is visible
    fn cursor_visible(&self) -> PyResult<bool> {
        Ok(self.inner.cursor().visible)
    }

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

    /// Get modifyOtherKeys mode (XTerm extension for enhanced keyboard input)
    ///
    /// Returns:
    ///     Current mode: 0=disabled, 1=report modifiers for special keys, 2=report all keys
    ///
    /// Example:
    ///     >>> term.modify_other_keys_mode()
    ///     0
    fn modify_other_keys_mode(&self) -> PyResult<u8> {
        Ok(self.inner.modify_other_keys_mode())
    }

    /// Set modifyOtherKeys mode (XTerm extension for enhanced keyboard input)
    ///
    /// Args:
    ///     mode: 0=disabled, 1=report modifiers for special keys, 2=report all keys
    ///
    /// Note:
    ///     Values > 2 are clamped to 2. This directly sets the mode without
    ///     sending escape sequences. Use process(b"\\x1b[>4;Nm") to set via sequence.
    ///
    /// Example:
    ///     >>> term.set_modify_other_keys_mode(2)
    ///     >>> term.modify_other_keys_mode()
    ///     2
    fn set_modify_other_keys_mode(&mut self, mode: u8) -> PyResult<()> {
        self.inner.set_modify_other_keys_mode(mode);
        Ok(())
    }

    // clipboard: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // set_clipboard: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    /// Check if clipboard read operations are allowed
    ///
    /// Returns:
    ///     True if OSC 52 queries (ESC ] 52 ; c ; ? ST) are allowed
    fn allow_clipboard_read(&self) -> PyResult<bool> {
        Ok(self.inner.allow_clipboard_read())
    }

    // set_allow_clipboard_read: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // default_fg: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_default_fg: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    /// Query default foreground color (OSC 10)
    ///
    /// Sends OSC 10 ; ? ST query and returns response in drain_responses().
    /// Response format: ESC ] 10 ; rgb:rrrr/gggg/bbbb ESC \
    fn query_default_fg(&mut self) -> PyResult<()> {
        self.inner.process(b"\x1b]10;?\x1b\\");
        Ok(())
    }

    // default_bg: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_default_bg: provided by impl_terminal_state_setters! (ARC-003/QA-001)

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

    // set_cursor_color: provided by impl_terminal_state_setters! (ARC-003/QA-001)

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

    // set_link_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_bold_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_cursor_guide_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_badge_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_match_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_selection_bg_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_selection_fg_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_use_bold_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // set_use_underline_color: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    /// Get link/hyperlink color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn link_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.link_color().to_rgb())
    }

    /// Get bold text color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn bold_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.bold_color().to_rgb())
    }

    /// Get cursor guide color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn cursor_guide_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.cursor_guide_color().to_rgb())
    }

    /// Get badge color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn badge_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.badge_color().to_rgb())
    }

    /// Get match/search highlight color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn match_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.match_color().to_rgb())
    }

    /// Get selection background color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn selection_bg_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.selection_bg_color().to_rgb())
    }

    /// Get selection foreground/text color
    ///
    /// Returns:
    ///     Tuple of (r, g, b) integers (0-255)
    fn selection_fg_color(&self) -> PyResult<(u8, u8, u8)> {
        Ok(self.inner.selection_fg_color().to_rgb())
    }

    /// Check if custom bold color is enabled
    ///
    /// Returns:
    ///     True if using custom bold color instead of bright ANSI variant
    fn use_bold_color(&self) -> PyResult<bool> {
        Ok(self.inner.use_bold_color())
    }

    /// Check if custom underline color is enabled
    ///
    /// Returns:
    ///     True if using custom underline color
    fn use_underline_color(&self) -> PyResult<bool> {
        Ok(self.inner.use_underline_color())
    }

    /// Check if bold brightening is enabled
    ///
    /// When enabled, bold text with ANSI colors 0-7 is brightened to 8-15.
    ///
    /// Returns:
    ///     True if bold brightening is enabled
    fn bold_brightening(&self) -> PyResult<bool> {
        Ok(self.inner.bold_brightening())
    }

    // set_bold_brightening: provided by impl_terminal_color_setters! (ARC-003/QA-001)

    // faint_text_alpha: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_faint_text_alpha: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // cursor_style: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // set_cursor_style: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // scrollback: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Get the number of scrollback lines
    ///
    /// Returns:
    ///     Number of lines in scrollback buffer
    fn scrollback_len(&self) -> PyResult<usize> {
        Ok(self.inner.grid().scrollback_len())
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
        if let Some(line) = self.inner.grid().row(row) {
            Ok(Some(
                line.iter()
                    .filter(|cell| !cell.flags.wide_char_spacer())
                    .map(|cell| cell.get_grapheme())
                    .collect::<Vec<String>>()
                    .join(""),
            ))
        } else {
            Ok(None)
        }
    }

    /// Get a cell's character at the specified position (includes combining characters/modifiers)
    ///
    /// Args:
    ///     col: Column index (0-based)
    ///     row: Row index (0-based)
    ///
    /// Returns:
    ///     Character (grapheme cluster) at the position, or None if out of bounds
    fn get_char(&self, col: usize, row: usize) -> PyResult<Option<String>> {
        if let Some(cell) = self.inner.active_grid().get(col, row) {
            Ok(Some(cell.get_grapheme()))
        } else {
            Ok(None)
        }
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
                        cell.get_grapheme(),
                        cell.fg.to_rgb(),
                        cell.bg.to_rgb(),
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

    // __str__: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // Advanced features

    // is_alt_screen_active: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Switch to alternate screen buffer
    ///
    /// This directly switches to the alternate screen without using escape sequences.
    /// The primary screen content is preserved and can be restored with use_primary_screen().
    /// Clears the alternate screen buffer.
    fn use_alt_screen(&mut self) -> PyResult<()> {
        self.inner.use_alt_screen();
        Ok(())
    }

    /// Switch to primary screen buffer
    ///
    /// This directly switches to the primary screen without using escape sequences.
    /// Restores the content that was visible before switching to alternate screen.
    /// Also resets keyboard protocol flags (for TUI apps that fail to clean up).
    fn use_primary_screen(&mut self) -> PyResult<()> {
        self.inner.use_primary_screen();
        Ok(())
    }

    // mouse_mode: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Get mouse encoding format
    ///
    /// Returns:
    ///     MouseEncoding enum value (Default, Utf8, Sgr, Urxvt)
    fn mouse_encoding(&self) -> PyResult<PyMouseEncoding> {
        Ok(self.inner.mouse_encoding().into())
    }

    /// Set mouse encoding format
    ///
    /// Controls how mouse events are encoded when reported to applications.
    ///
    /// Args:
    ///     encoding: MouseEncoding enum value
    ///         - Default: X11 encoding (values 32-255, limited coordinate range)
    ///         - Utf8: UTF-8 encoding (supports larger coordinates)
    ///         - Sgr: SGR encoding (1006) - recommended for modern terminals
    ///         - Urxvt: URXVT encoding (1015)
    fn set_mouse_encoding(&mut self, encoding: PyMouseEncoding) -> PyResult<()> {
        self.inner.set_mouse_encoding(encoding.into());
        Ok(())
    }

    // focus_tracking: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Set focus tracking mode
    ///
    /// When enabled, the terminal reports focus in/out events to applications.
    /// Focus events are reported as ESC[I (focus in) and ESC[O (focus out).
    ///
    /// Args:
    ///     enabled: True to enable focus tracking, False to disable
    fn set_focus_tracking(&mut self, enabled: bool) -> PyResult<()> {
        self.inner.set_focus_tracking(enabled);
        Ok(())
    }

    /// Check if bracketed paste mode is enabled
    ///
    /// Returns:
    ///     True if bracketed paste mode is enabled
    fn bracketed_paste(&self) -> PyResult<bool> {
        Ok(self.inner.bracketed_paste())
    }

    /// Set bracketed paste mode
    ///
    /// When enabled, pasted content is wrapped with ESC[200~ and ESC[201~
    /// sequences, allowing applications to distinguish pasted text from typed text.
    ///
    /// Args:
    ///     enabled: True to enable bracketed paste, False to disable
    fn set_bracketed_paste(&mut self, enabled: bool) -> PyResult<()> {
        self.inner.set_bracketed_paste(enabled);
        Ok(())
    }

    // synchronized_updates: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // flush_synchronized_updates: provided by impl_terminal_state_setters! (ARC-003/QA-001)

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

    // get_focus_in_event, get_focus_out_event, get_paste_start, get_paste_end:
    //   provided by impl_terminal_content_misc! (ARC-003/QA-001)

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

    // shell_integration_state: provided by impl_terminal_content_misc! (ARC-003/QA-001)

    // Sixel graphics methods
    // graphics_at_row, graphics_count, graphics, clear_graphics:
    //   provided by impl_terminal_sixel_graphics! (ARC-003/QA-001)

    /// Export all graphics metadata as a JSON string for session persistence
    ///
    /// Serializes all active placements, scrollback graphics, and animation state
    /// into a JSON string. Image pixel data is base64-encoded inline.
    ///
    /// Returns:
    ///     JSON string containing the serialized graphics snapshot
    ///
    /// Example:
    ///     >>> json_str = terminal.export_graphics_json()
    ///     >>> with open("session_graphics.json", "w") as f:
    ///     ...     f.write(json_str)
    fn export_graphics_json(&self) -> PyResult<String> {
        self.inner
            .graphics_store()
            .export_json()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Import graphics metadata from a JSON string to restore session state
    ///
    /// Deserializes graphics from JSON and restores active placements, scrollback
    /// graphics, and animation state. Existing graphics are cleared first.
    ///
    /// Args:
    ///     json: JSON string from a previous export_graphics_json() call
    ///
    /// Returns:
    ///     Number of graphics restored
    ///
    /// Example:
    ///     >>> with open("session_graphics.json") as f:
    ///     ...     json_str = f.read()
    ///     >>> count = terminal.import_graphics_json(json_str)
    ///     >>> print(f"Restored {count} graphics")
    fn import_graphics_json(&mut self, json: &str) -> PyResult<usize> {
        self.inner
            .graphics_store_mut()
            .import_json(json)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    // update_animations: provided by impl_terminal_exports! (ARC-003/QA-001)

    // Device query response methods

    // drain_responses, has_pending_responses, has_notifications, take_notifications,
    // drain_notifications, progress_bar, has_progress, progress_value, progress_state,
    // set_progress, clear_progress: provided by impl_terminal_progress_notifications! (ARC-003/QA-001)

    // Named progress bar methods (OSC 934)

    /// Get all named progress bars as a dictionary
    ///
    /// Returns a dictionary mapping progress bar IDs to their state.
    /// Each value is a dict with keys: id, state, percent, label.
    ///
    /// Returns:
    ///     Dictionary of {id: {id, state, percent, label}} for all active bars
    ///
    /// Example:
    ///     ```python
    ///     bars = term.named_progress_bars()
    ///     for bar_id, bar in bars.items():
    ///         print(f"{bar_id}: {bar['percent']}% - {bar.get('label', '')}")
    ///     ```
    fn named_progress_bars(&self) -> PyResult<HashMap<String, HashMap<String, String>>> {
        Ok(self
            .inner
            .named_progress_bars()
            .iter()
            .map(|(id, bar)| {
                let mut map = HashMap::new();
                map.insert("id".to_string(), bar.id.clone());
                map.insert("state".to_string(), bar.state.description().to_string());
                map.insert("percent".to_string(), bar.percent.to_string());
                if let Some(label) = &bar.label {
                    map.insert("label".to_string(), label.clone());
                }
                (id.clone(), map)
            })
            .collect())
    }

    /// Get a specific named progress bar by ID
    ///
    /// Args:
    ///     id: The progress bar identifier
    ///
    /// Returns:
    ///     Dict with keys: id, state, percent, label (optional), or None if not found
    fn get_named_progress_bar(&self, id: &str) -> PyResult<Option<HashMap<String, String>>> {
        Ok(self.inner.get_named_progress_bar(id).map(|bar| {
            let mut map = HashMap::new();
            map.insert("id".to_string(), bar.id.clone());
            map.insert("state".to_string(), bar.state.description().to_string());
            map.insert("percent".to_string(), bar.percent.to_string());
            if let Some(label) = &bar.label {
                map.insert("label".to_string(), label.clone());
            }
            map
        }))
    }

    /// Manually set or update a named progress bar
    ///
    /// Args:
    ///     id: Unique identifier for the progress bar
    ///     state: State string (normal, indeterminate, warning, error)
    ///     percent: Progress percentage (0-100, clamped if out of range)
    ///     label: Optional descriptive label
    #[pyo3(signature = (id, state="normal", percent=0, label=None))]
    fn set_named_progress_bar(
        &mut self,
        id: &str,
        state: &str,
        percent: u8,
        label: Option<String>,
    ) -> PyResult<()> {
        let progress_state = match state {
            "normal" => crate::terminal::ProgressState::Normal,
            "indeterminate" => crate::terminal::ProgressState::Indeterminate,
            "warning" => crate::terminal::ProgressState::Warning,
            "error" => crate::terminal::ProgressState::Error,
            "hidden" => crate::terminal::ProgressState::Hidden,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "Invalid state: {}. Valid: normal, indeterminate, warning, error, hidden",
                    state
                )));
            }
        };
        let bar =
            crate::terminal::NamedProgressBar::new(id.to_string(), progress_state, percent, label);
        self.inner.set_named_progress_bar(bar);
        Ok(())
    }

    /// Remove a named progress bar by ID
    ///
    /// Args:
    ///     id: The progress bar identifier to remove
    ///
    /// Returns:
    ///     True if the bar existed and was removed, False otherwise
    fn remove_named_progress_bar(&mut self, id: &str) -> PyResult<bool> {
        Ok(self.inner.remove_named_progress_bar(id))
    }

    /// Remove all named progress bars
    fn remove_all_named_progress_bars(&mut self) -> PyResult<()> {
        self.inner.remove_all_named_progress_bars();
        Ok(())
    }

    // debug_snapshot_buffer, debug_snapshot_grid, debug_snapshot_primary,
    // debug_snapshot_alt, debug_log_snapshot:
    //   provided by impl_terminal_debug_snapshots! (ARC-003/QA-001)

    // current_directory: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    /// Check if OSC 7 directory tracking is enabled
    ///
    /// Returns:
    ///     True if OSC 7 sequences are accepted, False otherwise
    fn accept_osc7(&self) -> PyResult<bool> {
        Ok(self.inner.accept_osc7())
    }

    // answerback_string: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_answerback_string: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // width_config: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_width_config: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // set_ambiguous_width: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // set_unicode_version: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    /// Get the current Unicode normalization form
    ///
    /// Returns:
    ///     NormalizationForm: The current normalization form (default: NFC)
    fn normalization_form(&self) -> PyResult<super::enums::PyNormalizationForm> {
        Ok(self.inner.normalization_form().into())
    }

    /// Set the Unicode normalization form
    ///
    /// Controls how Unicode text is normalized before being stored in cells.
    /// Default is NFC (Canonical Decomposition, followed by Canonical Composition).
    ///
    /// Args:
    ///     form: NormalizationForm enum value (e.g., NormalizationForm.NFC)
    ///
    /// Example:
    ///     >>> term.set_normalization_form(NormalizationForm.NFC)  # Compose characters
    ///     >>> term.set_normalization_form(NormalizationForm.NFD)  # Decompose characters
    ///     >>> term.set_normalization_form(NormalizationForm.None) # No normalization
    fn set_normalization_form(&mut self, form: super::enums::PyNormalizationForm) -> PyResult<()> {
        self.inner.set_normalization_form(form.into());
        Ok(())
    }

    // char_width: provided by impl_terminal_query_getters! (ARC-003/QA-001)

    // set_accept_osc7: provided by impl_terminal_state_setters! (ARC-003/QA-001)

    // disable_insecure_sequences, set_disable_insecure_sequences:
    //   provided by impl_terminal_content_misc! (ARC-003/QA-001)

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

    // ========== Static Utility Methods ==========
    // strip_ansi, measure_text_width, parse_color: provided by impl_terminal_static_helpers! (ARC-003/QA-001)
    // get_sixel_limits, set_sixel_limits, get_sixel_graphics_limit, set_sixel_graphics_limit,
    // get_dropped_sixel_graphics, get_sixel_stats: provided by impl_terminal_sixel_graphics! (ARC-003/QA-001)

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

    /// Enable or disable tmux control mode auto-detection
    ///
    /// When enabled, the parser will automatically switch to control mode
    /// when it sees a `%begin` notification from tmux. This helps handle
    /// race conditions where `set_tmux_control_mode(True)` is called after
    /// tmux has already started outputting control protocol.
    ///
    /// Note: Auto-detection is automatically enabled when
    /// `set_tmux_control_mode(True)` is called.
    ///
    /// Args:
    ///     enabled: True to enable auto-detection, False to disable
    ///
    /// Example:
    ///     ```python
    ///     term = Terminal(80, 24)
    ///     # Enable auto-detection before starting tmux
    ///     term.set_tmux_auto_detect(True)
    ///     # Terminal will automatically switch to control mode when %begin is seen
    ///     ```
    fn set_tmux_auto_detect(&mut self, enabled: bool) -> PyResult<()> {
        self.inner.set_tmux_auto_detect(enabled);
        Ok(())
    }

    /// Check if tmux control mode auto-detection is enabled
    ///
    /// Returns:
    ///     True if auto-detection is enabled, False otherwise
    fn is_tmux_auto_detect(&self) -> PyResult<bool> {
        Ok(self.inner.is_tmux_auto_detect())
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
    fn get_dirty_region(&self) -> PyResult<Option<(usize, usize, usize, usize)>> {
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
        Ok(self
            .inner
            .drain_bell_events()
            .iter()
            .map(|e| match e {
                BellEvent::VisualBell => "visual".to_string(),
                BellEvent::WarningBell(vol) => format!("warning:{}", vol),
                BellEvent::MarginBell(vol) => format!("margin:{}", vol),
            })
            .collect())
    }

    /// Drain all pending terminal events
    ///
    /// Returns and clears the buffer of terminal events.
    /// Events are returned as dictionaries with 'type' and additional fields.
    fn poll_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        use crate::python_bindings::observer::event_to_dict;
        let events = self.inner.poll_events();
        Ok(events.iter().map(event_to_dict).collect())
    }

    /// Drain pending screen cleared events
    ///
    /// Returns a list of booleans indicating whether each clear event also
    /// cleared the scrollback buffer (True for ESC[3J, False for ESC[2J).
    ///
    /// This is useful for frontends to invalidate scrollback zone/mark metadata
    /// so the scrollbar is consistent with the visible terminal state.
    ///
    /// Returns:
    ///     list[bool]: List of include_scrollback flags for each ScreenCleared event.
    ///
    /// Example:
    ///     >>> cleared = term.poll_screen_cleared_events()
    ///     >>> for include_scrollback in cleared:
    ///     ...     if include_scrollback:
    ///     ...         print("Screen and scrollback cleared (ESC[3J)")
    ///     ...     else:
    ///     ...         print("Screen cleared (ESC[2J)")
    fn poll_screen_cleared_events(&mut self) -> Vec<bool> {
        self.inner.poll_screen_cleared_events()
    }

    /// Set event subscription filter
    ///
    /// Args:
    ///     kinds: Optional list of event kinds to receive (strings).
    ///            Valid kinds: bell, title_changed, size_changed, mode_changed,
    ///            graphics_added, hyperlink_added, dirty_region, cwd_changed,
    ///            trigger_matched, user_var_changed, progress_bar_changed,
    ///            badge_changed, shell_integration, zone_opened, zone_closed,
    ///            zone_scrolled_out, environment_changed, remote_host_transition,
    ///            sub_shell_detected.
    #[pyo3(signature = (kinds=None))]
    fn set_event_subscription(&mut self, kinds: Option<Vec<String>>) -> PyResult<()> {
        let mapped = kinds.map(|items| {
            items
                .into_iter()
                .filter_map(|k| Self::parse_event_kind(&k))
                .collect()
        });
        self.inner
            .set_event_subscription(mapped.unwrap_or_default());
        Ok(())
    }

    /// Clear event subscription filter (equivalent to receiving all events)
    fn clear_event_subscription(&mut self) -> PyResult<()> {
        self.inner.clear_event_subscription();
        Ok(())
    }

    /// Register a synchronous observer callback
    ///
    /// The callback receives a dict for each terminal event.
    /// Returns an observer ID for later removal.
    ///
    /// Args:
    ///     callback: A Python callable that accepts a single dict argument.
    ///     kinds: Optional list of event kind strings to filter on.
    ///
    /// Returns:
    ///     int: A unique observer ID.
    ///
    /// Example:
    ///     >>> def on_event(event):
    ///     ...     print(event["type"])
    ///     >>> observer_id = term.add_observer(on_event, kinds=["bell", "title_changed"])
    #[pyo3(signature = (callback, kinds=None))]
    fn add_observer(
        &mut self,
        callback: Py<pyo3::types::PyAny>,
        kinds: Option<Vec<String>>,
    ) -> PyResult<u64> {
        use crate::python_bindings::observer::PyCallbackObserver;
        let subs = kinds.map(|items| {
            items
                .into_iter()
                .filter_map(|k| Self::parse_event_kind(&k))
                .collect()
        });
        let observer = std::sync::Arc::new(PyCallbackObserver::new(callback, subs));
        Ok(self.inner.add_observer(observer))
    }

    /// Register an async observer using an asyncio.Queue
    ///
    /// Creates an asyncio.Queue and registers an observer that pushes event dicts
    /// into it via `put_nowait()`. Returns both the observer ID and the queue.
    ///
    /// Args:
    ///     kinds: Optional list of event kind strings to filter on.
    ///
    /// Returns:
    ///     tuple[int, asyncio.Queue]: (observer_id, queue)
    ///
    /// Example:
    ///     >>> observer_id, queue = term.add_async_observer(kinds=["title_changed"])
    ///     >>> term.process(b"\x1b]0;Hello\x07")
    ///     >>> event = queue.get_nowait()
    #[pyo3(signature = (kinds=None))]
    fn add_async_observer(
        &mut self,
        py: Python<'_>,
        kinds: Option<Vec<String>>,
    ) -> PyResult<(u64, Py<pyo3::types::PyAny>)> {
        use crate::python_bindings::observer::PyQueueObserver;
        let asyncio = py.import("asyncio")?;
        let queue = asyncio.call_method0("Queue")?;
        let queue_obj: Py<pyo3::types::PyAny> = queue.unbind();
        let subs = kinds.map(|items| {
            items
                .into_iter()
                .filter_map(|k| Self::parse_event_kind(&k))
                .collect()
        });
        let queue_clone = queue_obj.clone_ref(py);
        let observer = std::sync::Arc::new(PyQueueObserver::new(queue_clone, subs));
        let id = self.inner.add_observer(observer);
        Ok((id, queue_obj))
    }

    /// Remove a previously registered observer
    ///
    /// Args:
    ///     observer_id: The ID returned by add_observer or add_async_observer.
    ///
    /// Returns:
    ///     bool: True if the observer was found and removed.
    fn remove_observer(&mut self, observer_id: u64) -> PyResult<bool> {
        Ok(self.inner.remove_observer(observer_id))
    }

    /// Get the number of currently registered observers
    ///
    /// Returns:
    ///     int: Number of observers.
    fn observer_count(&self) -> PyResult<usize> {
        Ok(self.inner.observer_count())
    }

    /// Drain events matching the current subscription
    ///
    /// Returns:
    ///     List of event dictionaries (same shape as poll_events)
    fn poll_subscribed_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        use crate::python_bindings::observer::event_to_dict;
        let events = self.inner.poll_subscribed_events();
        Ok(events.iter().map(event_to_dict).collect())
    }

    /// Drain only CWD change events
    ///
    /// Returns:
    ///     List of dicts: new_cwd, old_cwd (optional), hostname (optional),
    ///     username (optional), timestamp
    fn poll_cwd_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        let events = self.inner.poll_cwd_events();
        Ok(events
            .into_iter()
            .map(|change| {
                let mut map = HashMap::new();
                if let Some(old) = change.old_cwd {
                    map.insert("old_cwd".to_string(), old);
                }
                map.insert("new_cwd".to_string(), change.new_cwd);
                if let Some(host) = change.hostname {
                    map.insert("hostname".to_string(), host);
                }
                if let Some(user) = change.username {
                    map.insert("username".to_string(), user);
                }
                map.insert("timestamp".to_string(), change.timestamp.to_string());
                map
            })
            .collect())
    }

    /// Drain only shell integration events, keeping other events queued
    ///
    /// Returns events with their captured cursor_line so callers can process
    /// each marker at the correct absolute line (scrollback_len + cursor_row
    /// at the time the OSC 133 sequence was parsed).
    ///
    /// Returns:
    ///     List of dicts with keys: event_type, command, exit_code, timestamp, cursor_line
    fn poll_shell_integration_events(&mut self) -> PyResult<Vec<HashMap<String, String>>> {
        let events = self.inner.poll_shell_integration_events();
        Ok(events
            .into_iter()
            .map(|(event_type, command, exit_code, timestamp, cursor_line)| {
                let mut map = HashMap::new();
                map.insert("event_type".to_string(), event_type);
                if let Some(cmd) = command {
                    map.insert("command".to_string(), cmd);
                }
                if let Some(code) = exit_code {
                    map.insert("exit_code".to_string(), code.to_string());
                }
                if let Some(ts) = timestamp {
                    map.insert("timestamp".to_string(), ts.to_string());
                }
                if let Some(line) = cursor_line {
                    map.insert("cursor_line".to_string(), line.to_string());
                }
                map
            })
            .collect())
    }

    /// Drain only upload request events, keeping other events queued
    ///
    /// Returns:
    ///     List of format strings from pending UploadRequested events
    fn poll_upload_requests(&mut self) -> PyResult<Vec<String>> {
        Ok(self.inner.poll_upload_requests())
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
        Ok(Some(self.inner.left_right_margins()))
    }

    /// Get an ANSI palette color by index (0-15)
    fn get_ansi_color(&self, index: u8) -> PyResult<Option<(u8, u8, u8)>> {
        use crate::color::Color;
        if let Some(color) = self.inner.get_ansi_color(index as usize) {
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
        Ok(palette
            .iter()
            .map(|c| match c {
                Color::Rgb(r, g, b) => (*r, *g, *b),
                _ => (0, 0, 0), // Fallback for non-RGB colors
            })
            .collect())
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
    #[allow(clippy::type_complexity)]
    fn get_all_hyperlinks(&self) -> PyResult<Vec<(String, Vec<(usize, usize)>, Option<String>)>> {
        let links = self.inner.get_all_hyperlinks();
        Ok(links
            .iter()
            .map(|link| (link.url.clone(), link.positions.clone(), link.id.clone()))
            .collect())
    }

    /// Get a rectangular region of the screen
    ///
    /// Returns cells in rectangle bounded by (top, left) to (bottom, right) inclusive.
    /// Returns list of rows, where each row is a list of Cell dictionaries.
    fn get_rectangle(
        &self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> PyResult<Vec<Vec<HashMap<String, String>>>> {
        let cells = self.inner.get_rectangle(top, left, bottom, right);
        Ok(cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| {
                        let mut map = HashMap::new();
                        map.insert("char".to_string(), cell.c.to_string());
                        map.insert("width".to_string(), cell.width.to_string());
                        map
                    })
                    .collect()
            })
            .collect())
    }

    /// Fill a rectangle with a character
    fn fill_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
        ch: char,
    ) -> PyResult<()> {
        self.inner.fill_rectangle(top, left, bottom, right, ch);
        Ok(())
    }

    /// Erase a rectangle
    fn erase_rectangle(
        &mut self,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) -> PyResult<()> {
        self.inner.erase_rectangle(top, left, bottom, right);
        Ok(())
    }
}

impl PyTerminal {
    /// Parse an event kind string to `TerminalEventKind`.
    ///
    /// Returns `None` for unrecognised strings (silently ignored).
    fn parse_event_kind(kind: &str) -> Option<crate::terminal::TerminalEventKind> {
        use crate::terminal::TerminalEventKind;
        match kind {
            "bell" => Some(TerminalEventKind::BellRang),
            "title_changed" => Some(TerminalEventKind::TitleChanged),
            "size_changed" => Some(TerminalEventKind::SizeChanged),
            "mode_changed" => Some(TerminalEventKind::ModeChanged),
            "graphics_added" => Some(TerminalEventKind::GraphicsAdded),
            "hyperlink_added" => Some(TerminalEventKind::HyperlinkAdded),
            "dirty_region" => Some(TerminalEventKind::DirtyRegion),
            "cwd_changed" => Some(TerminalEventKind::CwdChanged),
            "trigger_matched" => Some(TerminalEventKind::TriggerMatched),
            "user_var_changed" => Some(TerminalEventKind::UserVarChanged),
            "progress_bar_changed" => Some(TerminalEventKind::ProgressBarChanged),
            "badge_changed" => Some(TerminalEventKind::BadgeChanged),
            "shell_integration" => Some(TerminalEventKind::ShellIntegrationEvent),
            "zone_opened" => Some(TerminalEventKind::ZoneOpened),
            "zone_closed" => Some(TerminalEventKind::ZoneClosed),
            "zone_scrolled_out" => Some(TerminalEventKind::ZoneScrolledOut),
            "environment_changed" => Some(TerminalEventKind::EnvironmentChanged),
            "remote_host_transition" => Some(TerminalEventKind::RemoteHostTransition),
            "sub_shell_detected" => Some(TerminalEventKind::SubShellDetected),
            "file_transfer_started" => Some(TerminalEventKind::FileTransferStarted),
            "file_transfer_progress" => Some(TerminalEventKind::FileTransferProgress),
            "file_transfer_completed" => Some(TerminalEventKind::FileTransferCompleted),
            "file_transfer_failed" => Some(TerminalEventKind::FileTransferFailed),
            "upload_requested" => Some(TerminalEventKind::UploadRequested),
            _ => None,
        }
    }
}

/// Helper function to parse clipboard slot from string
pub(super) fn parse_clipboard_slot(slot: &str) -> PyResult<crate::terminal::ClipboardSlot> {
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
            Err(PyValueError::new_err(
                "Invalid custom clipboard slot (use custom0-custom9)",
            ))
        }
        _ => Err(PyValueError::new_err("Invalid clipboard slot")),
    }
}

/// Convert a `FileTransfer` to a Python dictionary
///
/// Creates a `PyDict` with the transfer's metadata fields. When `include_data`
/// is true, also includes the raw file data as `PyBytes` under the `"data"` key.
pub(crate) fn transfer_to_py_dict(
    py: Python<'_>,
    transfer: &crate::terminal::file_transfer::FileTransfer,
    include_data: bool,
) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
    use crate::terminal::file_transfer::{TransferDirection, TransferStatus};
    use pyo3::types::{PyBytes, PyDict};

    let dict = PyDict::new(py);

    dict.set_item("id", transfer.id)?;

    let direction = match transfer.direction {
        TransferDirection::Download => "download",
        TransferDirection::Upload => "upload",
    };
    dict.set_item("direction", direction)?;

    // Convert empty filename to None
    let filename: Option<&str> = if transfer.filename.is_empty() {
        None
    } else {
        Some(&transfer.filename)
    };
    dict.set_item("filename", filename)?;

    let (status, bytes_transferred, total_bytes) = match &transfer.status {
        TransferStatus::Pending => ("pending", 0usize, None),
        TransferStatus::InProgress {
            bytes_transferred,
            total_bytes,
        } => ("in_progress", *bytes_transferred, *total_bytes),
        TransferStatus::Completed => ("completed", transfer.data.len(), Some(transfer.data.len())),
        TransferStatus::Failed(_) => ("failed", transfer.data.len(), None),
        TransferStatus::Cancelled => ("cancelled", transfer.data.len(), None),
    };
    dict.set_item("status", status)?;
    dict.set_item("bytes_transferred", bytes_transferred)?;
    dict.set_item("total_bytes", total_bytes)?;
    dict.set_item("started_at", transfer.started_at)?;
    dict.set_item("completed_at", transfer.completed_at)?;

    if include_data {
        let py_bytes = PyBytes::new(py, &transfer.data);
        dict.set_item("data", py_bytes)?;
    }

    Ok(dict.into())
}
