//! `PyScreenshotConfig` — a reusable options object for screenshot rendering
//! (QA-005), so callers don't have to repeat 16+ keyword arguments on every
//! `screenshot()` / `screenshot_to_file()` call.
//!
//! Build one, tweak the fields you care about, and pass it to
//! [`PyTerminal::screenshot_config`] / [`PyPtyTerminal::screenshot_config`].
//! The existing positional/keyword `screenshot()` signatures remain for
//! one-off use.

use pyo3::prelude::*;

use crate::screenshot::ScreenshotConfig;

use super::conversions::parse_sixel_mode;

/// Reusable screenshot rendering options (QA-005).
///
/// Pass to ``screenshot_config(config, scrollback_offset=0)`` /
/// ``screenshot_to_file_config(path, config, scrollback_offset=0)`` instead of
/// repeating the keyword arguments on every call.
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ScreenshotConfig", skip_from_py_object)]
#[derive(Clone)]
pub struct PyScreenshotConfig {
    /// Output image format: "png" | "jpeg" | "svg" | "bmp".
    pub(crate) format: String,
    /// Path to a .ttf/.otf font; None uses the embedded JetBrains Mono.
    pub(crate) font_path: Option<String>,
    /// Font size in pixels.
    pub(crate) font_size: f32,
    /// Include the scrollback buffer.
    pub(crate) include_scrollback: bool,
    /// Padding around the content in pixels.
    pub(crate) padding: u32,
    /// JPEG quality (1-100).
    pub(crate) quality: u8,
    /// Render the cursor.
    pub(crate) render_cursor: bool,
    /// Cursor color (None = white).
    pub(crate) cursor_color: Option<(u8, u8, u8)>,
    /// Sixel graphics mode: "disabled" | "pixels" | "halfblocks".
    pub(crate) sixel_mode: String,
    /// Hyperlink color (None = foreground).
    pub(crate) link_color: Option<(u8, u8, u8)>,
    /// Bold text color (None = foreground).
    pub(crate) bold_color: Option<(u8, u8, u8)>,
    /// Whether `bold_color` overrides the cell color.
    pub(crate) use_bold_color: bool,
    /// Bold brightening (ANSI 0-7 → bright 8-15 when bold).
    pub(crate) bold_brightening: bool,
    /// Background color override (None = terminal background).
    pub(crate) background_color: Option<(u8, u8, u8)>,
    /// Faint/dim text alpha (0.0-1.0).
    pub(crate) faint_text_alpha: f32,
    /// Minimum contrast adjustment (0.0-1.0).
    pub(crate) minimum_contrast: f64,
}

#[pymethods]
impl PyScreenshotConfig {
    /// Create a screenshot config with defaults (QA-005).
    ///
    /// All arguments are optional keyword arguments; only set the ones you
    /// need. Pass the result to ``screenshot_config`` / ``screenshot_to_file_config``.
    #[new]
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
        link_color = None,
        bold_color = None,
        use_bold_color = false,
        bold_brightening = false,
        background_color = None,
        faint_text_alpha = 0.5,
        minimum_contrast = 0.5
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        format: &str,
        font_path: Option<String>,
        font_size: f32,
        include_scrollback: bool,
        padding: u32,
        quality: u8,
        render_cursor: bool,
        cursor_color: Option<(u8, u8, u8)>,
        sixel_mode: &str,
        link_color: Option<(u8, u8, u8)>,
        bold_color: Option<(u8, u8, u8)>,
        use_bold_color: bool,
        bold_brightening: bool,
        background_color: Option<(u8, u8, u8)>,
        faint_text_alpha: f32,
        minimum_contrast: f64,
    ) -> Self {
        Self {
            format: format.to_string(),
            font_path,
            font_size,
            include_scrollback,
            padding,
            quality,
            render_cursor,
            cursor_color,
            sixel_mode: sixel_mode.to_string(),
            link_color,
            bold_color,
            use_bold_color,
            bold_brightening,
            background_color,
            faint_text_alpha,
            minimum_contrast,
        }
    }
}

impl PyScreenshotConfig {
    /// Build the Rust [`ScreenshotConfig`] from the public fields (QA-005).
    pub(crate) fn to_screenshot_config(&self) -> PyResult<ScreenshotConfig> {
        use crate::screenshot::ImageFormat;

        let img_format = match self.format.to_lowercase().as_str() {
            "png" => ImageFormat::Png,
            "jpeg" | "jpg" => ImageFormat::Jpeg,
            "svg" => ImageFormat::Svg,
            "bmp" => ImageFormat::Bmp,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "Invalid format: {}. Use png, jpeg, svg, or bmp",
                    other
                )));
            }
        };

        Ok(ScreenshotConfig {
            format: img_format,
            font_path: self.font_path.clone().map(std::path::PathBuf::from),
            font_size: self.font_size,
            include_scrollback: self.include_scrollback,
            padding_px: self.padding,
            quality: self.quality.min(100),
            render_cursor: self.render_cursor,
            cursor_color: self.cursor_color.unwrap_or((255, 255, 255)),
            sixel_render_mode: parse_sixel_mode(&self.sixel_mode)?,
            link_color: self.link_color,
            bold_color: self.bold_color,
            use_bold_color: self.use_bold_color,
            bold_brightening: self.bold_brightening,
            background_color: self.background_color,
            minimum_contrast: self.minimum_contrast.clamp(0.0, 1.0),
            faint_text_alpha: self.faint_text_alpha.clamp(0.0, 1.0),
            ..Default::default()
        })
    }
}
