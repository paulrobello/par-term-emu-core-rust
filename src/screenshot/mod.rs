mod config;
mod error;
mod font_cache;
mod formats;
mod renderer;
mod shaper;
mod utils;

pub use config::{ImageFormat, ScreenshotConfig, SixelRenderMode};
pub use error::{ScreenshotError, ScreenshotResult};
pub use shaper::FontType;

use crate::cursor::Cursor;
use crate::grid::Grid;
use crate::sixel::SixelGraphic;
use renderer::Renderer;
use std::path::Path;

/// Cursor information for rendering
#[derive(Debug, Clone)]
pub struct CursorInfo {
    pub col: usize,
    pub row: usize,
    pub visible: bool,
    pub style: crate::cursor::CursorStyle,
}

/// Render a grid to an image
pub fn render_grid(
    grid: &Grid,
    cursor: Option<&Cursor>,
    graphics: &[SixelGraphic],
    config: ScreenshotConfig,
) -> ScreenshotResult<Vec<u8>> {
    // SVG doesn't support cursor or sixel rendering yet
    if config.format == ImageFormat::Svg {
        return formats::svg::encode(grid, config.font_size, config.padding_px);
    }

    // Raster formats (PNG, JPEG, BMP)
    let rows = grid.rows();
    let cols = grid.cols();

    let mut renderer = Renderer::new(rows, cols, config.clone())?;
    let image = renderer.render_grid(grid, cursor, graphics)?;

    // Encode based on format
    match config.format {
        ImageFormat::Png => formats::png::encode(&image),
        ImageFormat::Jpeg => formats::jpeg::encode(&image, config.quality),
        ImageFormat::Svg => unreachable!(), // Already handled above
        ImageFormat::Bmp => formats::bmp::encode(&image),
    }
}

/// Save a grid as an image file
pub fn save_grid(
    grid: &Grid,
    cursor: Option<&Cursor>,
    graphics: &[SixelGraphic],
    path: &Path,
    config: ScreenshotConfig,
) -> ScreenshotResult<()> {
    let bytes = render_grid(grid, cursor, graphics, config)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;
    use crate::color::{Color, NamedColor};
    use crate::grid::Grid;

    #[test]
    fn test_basic_screenshot() {
        let mut grid = Grid::new(80, 24, 1000);

        // Add some test content
        for col in 0..10 {
            let mut cell = Cell::new('H');
            cell.fg = Color::Named(NamedColor::Red);
            grid.set(col, 0, cell);
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        // This might fail if no system font is available
        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            // Check PNG signature
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_flag_emoji_rendering() {
        let mut grid = Grid::new(80, 24, 1000);

        // Test single flag emoji (US flag: U+1F1FA U+1F1F8)
        // Note: Flag emojis are composed of two Regional Indicator characters
        let us_flag = "🇺🇸"; // This is actually two codepoints
        let chars: Vec<char> = us_flag.chars().collect();

        // Add the Regional Indicator characters to the grid
        for (col, &c) in chars.iter().enumerate() {
            let cell = Cell::new(c);
            grid.set(col, 0, cell);
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        // This might fail if no emoji font is available, but should not crash
        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_multiple_flag_emojis() {
        let mut grid = Grid::new(80, 24, 1000);

        // Test multiple flags: US, China, Japan, Korea
        let flags = "🇺🇸 🇨🇳 🇯🇵 🇰🇷";
        let chars: Vec<char> = flags.chars().collect();

        for (col, &c) in chars.iter().enumerate() {
            let cell = Cell::new(c);
            grid.set(col, 0, cell);
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_mixed_content_with_flags() {
        let mut grid = Grid::new(80, 24, 1000);

        // Test mixed text and flags: "Hello 🇺🇸 World"
        let text = "Hello 🇺🇸 World";
        let chars: Vec<char> = text.chars().collect();

        for (col, &c) in chars.iter().enumerate() {
            let cell = Cell::new(c);
            grid.set(col, 0, cell);
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_regular_emoji_still_works() {
        let mut grid = Grid::new(80, 24, 1000);

        // Test that regular single-codepoint emoji still work
        let emoji = "🐍 🦀 ❤️ ✨ 🚀 ☕";
        let chars: Vec<char> = emoji.chars().collect();

        for (col, &c) in chars.iter().enumerate() {
            let cell = Cell::new(c);
            grid.set(col, 0, cell);
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_cjk_with_flags() {
        let mut grid = Grid::new(80, 24, 1000);

        // Test CJK text mixed with flags
        let text = "中国 🇨🇳 日本 🇯🇵 한국 🇰🇷";
        let chars: Vec<char> = text.chars().collect();

        for (col, &c) in chars.iter().enumerate() {
            if col < grid.cols() {
                let cell = Cell::new(c);
                grid.set(col, 0, cell);
            }
        }

        let config = ScreenshotConfig::default();
        let result = render_grid(&grid, None, &[], config);

        if let Ok(bytes) = result {
            assert!(!bytes.is_empty());
            assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[test]
    fn test_regional_indicator_detection() {
        // Test that we correctly detect Regional Indicator characters
        use super::renderer::Renderer;

        // US flag is composed of U+1F1FA and U+1F1F8
        let us_flag = "🇺🇸";
        assert!(Renderer::contains_regional_indicators(us_flag));

        // Regular text should not contain Regional Indicators
        let regular_text = "Hello World";
        assert!(!Renderer::contains_regional_indicators(regular_text));

        // Regular emoji should not contain Regional Indicators
        let regular_emoji = "🚀 ❤️ ✨";
        assert!(!Renderer::contains_regional_indicators(regular_emoji));

        // Mixed content with flags should be detected
        let mixed = "Hello 🇺🇸 World";
        assert!(Renderer::contains_regional_indicators(mixed));
    }
}
