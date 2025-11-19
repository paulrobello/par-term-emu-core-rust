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
    /// Enable bold brightening (bold text with ANSI colors 0-7 uses bright variants 8-15)
    pub bold_brightening: bool,

    // Contrast settings
    /// Minimum contrast adjustment (0.0-1.0, iTerm2-compatible)
    /// 0.0 = disabled, 0.5 = moderate, 1.0 = maximum
    /// Default: 0.5 (moderate contrast for improved readability)
    /// Automatically adjusts text colors to maintain readability against backgrounds
    pub minimum_contrast: f64,
    /// Alpha multiplier for faint/dim text (0.0 = fully transparent, 1.0 = no dimming)
    /// Matches iTerm2's "Faint text" slider. Default: 0.5 (50%).
    pub faint_text_alpha: f32,
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
            bold_brightening: false,
            minimum_contrast: 0.5, // Moderate contrast by default (0.5 = 50% adjustment)
            faint_text_alpha: 0.5,
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

    /// Enable/disable bold brightening
    pub fn with_bold_brightening(mut self, enabled: bool) -> Self {
        self.bold_brightening = enabled;
        self
    }

    /// Set minimum contrast adjustment (0.0-1.0)
    /// 0.0 = disabled, 0.5 = moderate, 1.0 = maximum
    pub fn with_minimum_contrast(mut self, contrast: f64) -> Self {
        self.minimum_contrast = contrast.clamp(0.0, 1.0);
        self
    }

    /// Set faint text alpha (dim strength)
    pub fn with_faint_text_alpha(mut self, alpha: f32) -> Self {
        self.faint_text_alpha = alpha.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ScreenshotConfig::default();
        assert_eq!(config.font_path, None);
        assert_eq!(config.font_size, 14.0);
        assert_eq!(config.line_height_multiplier, 1.2);
        assert_eq!(config.char_width_multiplier, 1.0);
        assert!(!config.include_scrollback);
        assert_eq!(config.scrollback_lines, None);
        assert!(config.antialiasing);
        assert_eq!(config.padding_px, 10);
        assert_eq!(config.background_color, None);
        assert_eq!(config.format, ImageFormat::Png);
        assert_eq!(config.quality, 90);
        assert!(!config.render_cursor);
        assert_eq!(config.cursor_color, (255, 255, 255));
        assert_eq!(config.sixel_render_mode, SixelRenderMode::HalfBlocks);
        assert_eq!(config.link_color, None);
        assert_eq!(config.bold_color, None);
        assert!(!config.use_bold_color);
        assert!(!config.bold_brightening);
    }

    #[test]
    fn test_new_config() {
        let config = ScreenshotConfig::new();
        let default_config = ScreenshotConfig::default();
        assert_eq!(config.font_size, default_config.font_size);
        assert_eq!(config.format, default_config.format);
    }

    #[test]
    fn test_with_font_path() {
        let path = PathBuf::from("/path/to/font.ttf");
        let config = ScreenshotConfig::new().with_font_path(path.clone());
        assert_eq!(config.font_path, Some(path));
    }

    #[test]
    fn test_with_font_size() {
        let config = ScreenshotConfig::new().with_font_size(20.0);
        assert_eq!(config.font_size, 20.0);
    }

    #[test]
    fn test_with_format() {
        let config = ScreenshotConfig::new().with_format(ImageFormat::Jpeg);
        assert_eq!(config.format, ImageFormat::Jpeg);

        let config = ScreenshotConfig::new().with_format(ImageFormat::Svg);
        assert_eq!(config.format, ImageFormat::Svg);

        let config = ScreenshotConfig::new().with_format(ImageFormat::Bmp);
        assert_eq!(config.format, ImageFormat::Bmp);
    }

    #[test]
    fn test_with_scrollback() {
        let config = ScreenshotConfig::new().with_scrollback(true);
        assert!(config.include_scrollback);

        let config = ScreenshotConfig::new().with_scrollback(false);
        assert!(!config.include_scrollback);
    }

    #[test]
    fn test_with_padding() {
        let config = ScreenshotConfig::new().with_padding(20);
        assert_eq!(config.padding_px, 20);

        let config = ScreenshotConfig::new().with_padding(0);
        assert_eq!(config.padding_px, 0);
    }

    #[test]
    fn test_with_quality() {
        let config = ScreenshotConfig::new().with_quality(75);
        assert_eq!(config.quality, 75);
    }

    #[test]
    fn test_with_quality_clamped() {
        // Quality should be clamped to max 100
        let config = ScreenshotConfig::new().with_quality(150);
        assert_eq!(config.quality, 100);

        let config = ScreenshotConfig::new().with_quality(255);
        assert_eq!(config.quality, 100);
    }

    #[test]
    fn test_with_quality_minimum() {
        let config = ScreenshotConfig::new().with_quality(1);
        assert_eq!(config.quality, 1);
    }

    #[test]
    fn test_with_cursor() {
        let config = ScreenshotConfig::new().with_cursor(true);
        assert!(config.render_cursor);

        let config = ScreenshotConfig::new().with_cursor(false);
        assert!(!config.render_cursor);
    }

    #[test]
    fn test_with_sixel_mode() {
        let config = ScreenshotConfig::new().with_sixel_mode(SixelRenderMode::Disabled);
        assert_eq!(config.sixel_render_mode, SixelRenderMode::Disabled);

        let config = ScreenshotConfig::new().with_sixel_mode(SixelRenderMode::Pixels);
        assert_eq!(config.sixel_render_mode, SixelRenderMode::Pixels);

        let config = ScreenshotConfig::new().with_sixel_mode(SixelRenderMode::HalfBlocks);
        assert_eq!(config.sixel_render_mode, SixelRenderMode::HalfBlocks);
    }

    #[test]
    fn test_with_link_color() {
        let config = ScreenshotConfig::new().with_link_color((0, 0, 255));
        assert_eq!(config.link_color, Some((0, 0, 255)));
    }

    #[test]
    fn test_with_bold_color() {
        let config = ScreenshotConfig::new().with_bold_color((255, 0, 0));
        assert_eq!(config.bold_color, Some((255, 0, 0)));
    }

    #[test]
    fn test_with_use_bold_color() {
        let config = ScreenshotConfig::new().with_use_bold_color(true);
        assert!(config.use_bold_color);

        let config = ScreenshotConfig::new().with_use_bold_color(false);
        assert!(!config.use_bold_color);
    }

    #[test]
    fn test_builder_pattern_chaining() {
        let config = ScreenshotConfig::new()
            .with_font_size(16.0)
            .with_format(ImageFormat::Jpeg)
            .with_quality(85)
            .with_scrollback(true)
            .with_padding(15)
            .with_cursor(true)
            .with_sixel_mode(SixelRenderMode::Pixels)
            .with_link_color((0, 100, 200))
            .with_bold_color((200, 0, 0))
            .with_use_bold_color(true)
            .with_bold_brightening(true);

        assert_eq!(config.font_size, 16.0);
        assert_eq!(config.format, ImageFormat::Jpeg);
        assert_eq!(config.quality, 85);
        assert!(config.include_scrollback);
        assert_eq!(config.padding_px, 15);
        assert!(config.render_cursor);
        assert_eq!(config.sixel_render_mode, SixelRenderMode::Pixels);
        assert_eq!(config.link_color, Some((0, 100, 200)));
        assert_eq!(config.bold_color, Some((200, 0, 0)));
        assert!(config.use_bold_color);
        assert!(config.bold_brightening);
    }

    #[test]
    fn test_image_format_default() {
        assert_eq!(ImageFormat::default(), ImageFormat::Png);
    }

    #[test]
    fn test_image_format_equality() {
        assert_eq!(ImageFormat::Png, ImageFormat::Png);
        assert_eq!(ImageFormat::Jpeg, ImageFormat::Jpeg);
        assert_eq!(ImageFormat::Svg, ImageFormat::Svg);
        assert_eq!(ImageFormat::Bmp, ImageFormat::Bmp);
        assert_ne!(ImageFormat::Png, ImageFormat::Jpeg);
    }

    #[test]
    fn test_sixel_render_mode_equality() {
        assert_eq!(SixelRenderMode::Disabled, SixelRenderMode::Disabled);
        assert_eq!(SixelRenderMode::Pixels, SixelRenderMode::Pixels);
        assert_eq!(SixelRenderMode::HalfBlocks, SixelRenderMode::HalfBlocks);
        assert_ne!(SixelRenderMode::Pixels, SixelRenderMode::HalfBlocks);
    }

    #[test]
    fn test_config_clone() {
        let config = ScreenshotConfig::new()
            .with_font_size(18.0)
            .with_format(ImageFormat::Svg);

        let cloned = config.clone();
        assert_eq!(cloned.font_size, 18.0);
        assert_eq!(cloned.format, ImageFormat::Svg);
    }

    #[test]
    fn test_config_debug() {
        let config = ScreenshotConfig::new();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("ScreenshotConfig"));
    }

    #[test]
    fn test_with_bold_brightening() {
        let config = ScreenshotConfig::new().with_bold_brightening(true);
        assert!(config.bold_brightening);

        let config = ScreenshotConfig::new().with_bold_brightening(false);
        assert!(!config.bold_brightening);
    }

    #[test]
    fn test_default_minimum_contrast() {
        let config = ScreenshotConfig::default();
        assert_eq!(config.minimum_contrast, 0.5);
    }

    #[test]
    fn test_with_minimum_contrast() {
        let config = ScreenshotConfig::new().with_minimum_contrast(0.5);
        assert_eq!(config.minimum_contrast, 0.5);

        let config = ScreenshotConfig::new().with_minimum_contrast(1.0);
        assert_eq!(config.minimum_contrast, 1.0);
    }

    #[test]
    fn test_with_minimum_contrast_clamping() {
        // Values should be clamped to [0.0, 1.0]
        let config = ScreenshotConfig::new().with_minimum_contrast(1.5);
        assert_eq!(config.minimum_contrast, 1.0);

        let config = ScreenshotConfig::new().with_minimum_contrast(-0.5);
        assert_eq!(config.minimum_contrast, 0.0);
    }

    #[test]
    fn test_default_faint_text_alpha() {
        let config = ScreenshotConfig::default();
        assert!((config.faint_text_alpha - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_with_faint_text_alpha() {
        let config = ScreenshotConfig::new().with_faint_text_alpha(0.25);
        assert!((config.faint_text_alpha - 0.25).abs() < f32::EPSILON);

        let config = ScreenshotConfig::new().with_faint_text_alpha(1.5);
        assert!((config.faint_text_alpha - 1.0).abs() < f32::EPSILON);

        let config = ScreenshotConfig::new().with_faint_text_alpha(-0.5);
        assert!((config.faint_text_alpha - 0.0).abs() < f32::EPSILON);
    }
}
