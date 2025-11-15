use std::sync::Arc;
use swash::shape::ShapeContext;
use swash::text::Script;
use swash::FontRef;

use super::error::ScreenshotResult;
use super::utils::{is_cjk, is_emoji};

/// A shaped glyph with positioning information
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    /// Glyph ID in the font
    pub glyph_id: u32,
    /// Character cluster this glyph belongs to (maps back to character index)
    pub cluster: u32,
    /// Horizontal offset from the current position
    pub x_offset: i32,
    /// Vertical offset from the current position
    pub y_offset: i32,
    /// Horizontal advance to next glyph position
    #[allow(dead_code)]
    pub x_advance: i32,
    /// Vertical advance to next glyph position (usually 0 for LTR text)
    #[allow(dead_code)]
    pub y_advance: i32,
}

/// Font type indicator for shaped text
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontType {
    Regular,
    Emoji,
    Cjk,
}

/// A shaped glyph with its font source
#[derive(Clone, Debug)]
pub struct ShapedGlyphWithFont {
    pub glyph: ShapedGlyph,
    pub font_type: FontType,
}

/// Text shaper using swash
pub struct TextShaper {
    /// Font data for regular text (must be kept alive)
    regular_data: Arc<Vec<u8>>,
    /// Font data for emoji (must be kept alive)
    emoji_data: Option<Arc<Vec<u8>>>,
    /// Font data for CJK (must be kept alive)
    cjk_data: Option<Arc<Vec<u8>>>,
    /// Shape context for efficient shaping (reused across calls)
    shape_context: ShapeContext,
}

impl TextShaper {
    /// Create a new text shaper with regular font data
    pub fn new(font_data: Arc<Vec<u8>>) -> ScreenshotResult<Self> {
        Ok(Self {
            regular_data: font_data,
            emoji_data: None,
            cjk_data: None,
            shape_context: ShapeContext::new(),
        })
    }

    /// Add emoji font data
    pub fn set_emoji_font(&mut self, font_data: Arc<Vec<u8>>) -> ScreenshotResult<()> {
        self.emoji_data = Some(font_data);
        Ok(())
    }

    /// Add CJK font data
    pub fn set_cjk_font(&mut self, font_data: Arc<Vec<u8>>) -> ScreenshotResult<()> {
        self.cjk_data = Some(font_data);
        Ok(())
    }

    /// Determine which font should be used for a character
    fn select_font_for_char(&self, c: char) -> FontType {
        if is_cjk(c) && self.cjk_data.is_some() {
            FontType::Cjk
        } else if is_emoji(c) && self.emoji_data.is_some() {
            FontType::Emoji
        } else {
            FontType::Regular
        }
    }

    /// Get font data for a given font type
    fn get_font_data(&self, font_type: FontType) -> &Arc<Vec<u8>> {
        match font_type {
            FontType::Regular => &self.regular_data,
            FontType::Emoji => self.emoji_data.as_ref().unwrap_or(&self.regular_data),
            FontType::Cjk => self.cjk_data.as_ref().unwrap_or(&self.regular_data),
        }
    }

    /// Shape a line of text, breaking it into font runs
    pub fn shape_line(&mut self, text: &str) -> Vec<ShapedGlyphWithFont> {
        if text.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let chars: Vec<char> = text.chars().collect();

        // Split text into runs of same font type
        let mut current_run_start = 0;
        let mut current_font_type = self.select_font_for_char(chars[0]);

        for (i, &c) in chars.iter().enumerate().skip(1) {
            let font_type = self.select_font_for_char(c);

            // Check if we need to start a new run (font type changed)
            if font_type != current_font_type {
                // End current run
                let run_text: String = chars[current_run_start..i].iter().collect();
                // Calculate byte offset for this run (not character offset!)
                let byte_offset = chars[0..current_run_start]
                    .iter()
                    .map(|c| c.len_utf8())
                    .sum();
                let shaped = self.shape_run(&run_text, current_font_type, byte_offset);
                result.extend(shaped);

                // Start new run
                current_run_start = i;
                current_font_type = font_type;
            }
        }

        // Handle final run
        if current_run_start < chars.len() {
            let run_text: String = chars[current_run_start..].iter().collect();
            // Calculate byte offset for final run
            let byte_offset = chars[0..current_run_start]
                .iter()
                .map(|c| c.len_utf8())
                .sum();
            let shaped = self.shape_run(&run_text, current_font_type, byte_offset);
            result.extend(shaped);
        }

        result
    }

    /// Shape a single run of text with a specific font
    fn shape_run(
        &mut self,
        text: &str,
        font_type: FontType,
        cluster_offset: usize,
    ) -> Vec<ShapedGlyphWithFont> {
        // Clone the Arc to avoid borrow issues
        let font_data = self.get_font_data(font_type).clone();
        let font_ref = match FontRef::from_index(&font_data, 0) {
            Some(f) => f,
            None => return Vec::new(), // Font loading failed
        };

        // Create a shaper with the font
        let mut shaper = self
            .shape_context
            .builder(font_ref)
            .script(Script::Latin) // Default script, swash auto-detects
            .build();

        // Add text to the shaper
        shaper.add_str(text);

        // Shape and collect glyphs
        let mut shaped_glyphs = Vec::new();
        shaper.shape_with(|cluster| {
            // Convert swash cluster to our ShapedGlyph format
            for glyph in cluster.glyphs {
                shaped_glyphs.push(ShapedGlyphWithFont {
                    glyph: ShapedGlyph {
                        glyph_id: glyph.id as u32,
                        cluster: (cluster.source.start as usize + cluster_offset) as u32,
                        x_offset: glyph.x as i32,
                        y_offset: glyph.y as i32,
                        x_advance: glyph.advance as i32,
                        y_advance: 0, // Swash doesn't provide y_advance for horizontal text
                    },
                    font_type,
                });
            }
        });

        shaped_glyphs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shaper_creation() {
        // Use embedded font for testing
        const DEFAULT_FONT: &[u8] = include_bytes!("JetBrainsMono-Regular.ttf");
        let font_data = Arc::new(DEFAULT_FONT.to_vec());

        let shaper = TextShaper::new(font_data);
        assert!(shaper.is_ok());
    }

    #[test]
    fn test_basic_shaping() {
        const DEFAULT_FONT: &[u8] = include_bytes!("JetBrainsMono-Regular.ttf");
        let font_data = Arc::new(DEFAULT_FONT.to_vec());

        if let Ok(mut shaper) = TextShaper::new(font_data) {
            let shaped = shaper.shape_line("Hello");
            assert!(shaped.len() >= 5); // Should have at least 5 glyphs for "Hello"

            // All should be from regular font
            for glyph in &shaped {
                assert_eq!(glyph.font_type, FontType::Regular);
            }
        }
    }

    #[test]
    fn test_emoji_detection() {
        assert!(is_emoji('🚀'));
        assert!(is_emoji('❤'));
        assert!(!is_emoji('A'));
        assert!(!is_emoji('中'));
    }

    #[test]
    fn test_cjk_detection() {
        assert!(is_cjk('中'));
        assert!(is_cjk('日'));
        assert!(is_cjk('한'));
        assert!(!is_cjk('A'));
        assert!(!is_cjk('🚀'));
    }
}
