use std::path::PathBuf;

/// Image format for screenshot output
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFormat {
    /// PNG format (lossless)
    #[default]
    Png,
    /// JPEG format (lossy)
    Jpeg,
    /// SVG format (vector)
    Svg,
    /// BMP format (uncompressed)
    Bmp,
}

/// Sixel rendering mode for screenshots
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SixelRenderMode {
    /// Don't render Sixel graphics
    Disabled,
    /// Render Sixel graphics as actual pixels (shows the real image data)
    Pixels,
    /// Render Sixel graphics using half-block characters (matches TUI appearance)
    HalfBlocks,
}

/// Configuration for screenshot rendering
#[derive(Debug, Clone)]
pub struct ScreenshotConfig {
    // Font settings
    /// Path to custom font file (.ttf or .otf). None uses embedded default
    pub font_path: Option<PathBuf>,
    /// Font size in pixels
    pub font_size: f32,
    /// Line height multiplier (1.0 = tight, 1.2 = comfortable)
    pub line_height_multiplier: f32,
    /// Character width multiplier for spacing
    pub char_width_multiplier: f32,

    // Rendering options
    /// Include scrollback buffer in screenshot
    pub include_scrollback: bool,
    /// Number of scrollback lines to include (None = all)
    pub scrollback_lines: Option<usize>,
    /// Enable font antialiasing
    pub antialiasing: bool,

    // Canvas settings
    /// Padding around content in pixels
    pub padding_px: u32,
    /// Background color override (None = use terminal background)
    pub background_color: Option<(u8, u8, u8)>,

    // Output format
    /// Image format
    pub format: ImageFormat,
    /// JPEG quality (1-100)
    pub quality: u8,

    // Advanced options
    /// Render cursor in screenshot
    pub render_cursor: bool,
    /// Cursor color (RGB)
    pub cursor_color: (u8, u8, u8),
    /// Sixel graphics rendering mode
    pub sixel_render_mode: SixelRenderMode,

    // Theme colors
    /// Link/hyperlink color (None = use cell's foreground color)
    pub link_color: Option<(u8, u8, u8)>,
    /// Bold text custom color (None = use cell's foreground color)
    pub bold_color: Option<(u8, u8, u8)>,
    /// Use custom bold color instead of cell's color
    pub use_bold_color: bool,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            font_path: None,
            font_size: 14.0,
            line_height_multiplier: 1.2,
            char_width_multiplier: 1.0,
            include_scrollback: false,
            scrollback_lines: None,
            antialiasing: true,
            padding_px: 10,
            background_color: None,
            format: ImageFormat::Png,
            quality: 90,
            render_cursor: false,
            cursor_color: (255, 255, 255),
            sixel_render_mode: SixelRenderMode::HalfBlocks,
            link_color: None,
            bold_color: None,
            use_bold_color: false,
        }
    }
}

impl ScreenshotConfig {
    /// Create a new config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set custom font path
    pub fn with_font_path(mut self, path: PathBuf) -> Self {
        self.font_path = Some(path);
        self
    }

    /// Set font size
    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set image format
    pub fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = format;
        self
    }

    /// Include scrollback buffer
    pub fn with_scrollback(mut self, include: bool) -> Self {
        self.include_scrollback = include;
        self
    }

    /// Set padding
    pub fn with_padding(mut self, padding: u32) -> Self {
        self.padding_px = padding;
        self
    }

    /// Set JPEG quality
    pub fn with_quality(mut self, quality: u8) -> Self {
        self.quality = quality.min(100);
        self
    }

    /// Enable cursor rendering
    pub fn with_cursor(mut self, render: bool) -> Self {
        self.render_cursor = render;
        self
    }

    /// Set Sixel graphics rendering mode
    pub fn with_sixel_mode(mut self, mode: SixelRenderMode) -> Self {
        self.sixel_render_mode = mode;
        self
    }

    /// Set link/hyperlink color
    pub fn with_link_color(mut self, color: (u8, u8, u8)) -> Self {
        self.link_color = Some(color);
        self
    }

    /// Set bold text custom color
    pub fn with_bold_color(mut self, color: (u8, u8, u8)) -> Self {
        self.bold_color = Some(color);
        self
    }

    /// Enable/disable custom bold color
    pub fn with_use_bold_color(mut self, use_bold: bool) -> Self {
        self.use_bold_color = use_bold;
        self
    }
}
