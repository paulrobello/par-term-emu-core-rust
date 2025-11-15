use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::FontRef;

use super::error::{ScreenshotError, ScreenshotResult};
use super::utils::{is_cjk, is_emoji};

// Embedded default font (JetBrains Mono Regular)
// This is a high-quality monospace font with:
// - Programming ligatures (=>, !=, >=, etc.)
// - Full box drawing character support
// - Excellent Unicode coverage
// - Clean, modern design
// License: OFL-1.1 (SIL Open Font License)
// Size: ~268 KB
const DEFAULT_FONT: &[u8] = include_bytes!("JetBrainsMono-Regular.ttf");

// Embedded emoji fallback font (Noto Emoji Regular)
// This is a monochrome outline emoji font that provides:
// - Universal emoji coverage (all Unicode emoji)
// - Works on all platforms without system fonts
// - Decent grayscale styling (better than tofu boxes)
// - Used as final fallback when system color emoji fonts not found
// License: OFL-1.1 (SIL Open Font License)
// Size: ~409 KB
const EMOJI_FALLBACK_FONT: &[u8] = include_bytes!("NotoEmoji-Regular.ttf");

/// Bitmap format for cached glyphs
#[derive(Clone, Debug)]
pub enum BitmapFormat {
    /// Grayscale bitmap (8-bit alpha)
    Grayscale,
    /// RGBA bitmap (32-bit color with alpha)
    Rgba,
}

/// Glyph metrics matching fontdue's Metrics structure
#[derive(Clone, Debug)]
pub struct GlyphMetrics {
    #[allow(dead_code)]
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance_width: f32,
    #[allow(dead_code)]
    pub advance_height: f32,
}

/// Cached glyph data
#[derive(Clone)]
pub struct CachedGlyph {
    /// Glyph metrics (width, height, etc.)
    pub metrics: GlyphMetrics,
    /// Bitmap data (grayscale or RGBA)
    pub bitmap: Vec<u8>,
    /// Bitmap format
    pub format: BitmapFormat,
}

/// Font wrapper that holds swash font data
struct SwashFont {
    data: Arc<Vec<u8>>, // Keep data alive for FontRef
}

impl SwashFont {
    fn new(data: Vec<u8>) -> ScreenshotResult<Self> {
        let data = Arc::new(data);
        // Validate that the font can be loaded
        FontRef::from_index(&data, 0)
            .ok_or_else(|| ScreenshotError::FontLoadError("Failed to load font".to_string()))?;
        Ok(Self { data })
    }

    /// Get a FontRef for this font
    fn font_ref(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, 0)
    }

    /// Get the font data (for sharing with shaper)
    fn font_data(&self) -> &Arc<Vec<u8>> {
        &self.data
    }
}

/// Font cache for rendering glyphs
pub struct FontCache {
    /// Regular font (JetBrains Mono or custom)
    regular: SwashFont,
    /// Optional emoji fallback font (lazy-loaded from system)
    emoji_font: Option<SwashFont>,
    /// CJK fallback fonts (lazy-loaded from system, ordered by priority)
    cjk_fonts: Vec<SwashFont>,
    /// Cache mapping characters to their font index in cjk_fonts
    cjk_font_cache: HashMap<char, usize>,
    /// Glyph cache: (char, size, bold, italic) -> glyph
    cache: HashMap<(char, u32, bool, bool), CachedGlyph>,
    /// Font size in pixels
    font_size: f32,
    /// Cached cell dimensions (width, height)
    cell_dimensions: Option<(u32, u32)>,
    /// Cached font metrics (ascender, descender, height)
    font_metrics: Option<(i32, i32, i32)>,
    /// Scale context for rasterizing glyphs (reused for performance)
    scaler: ScaleContext,
}

impl FontCache {
    /// Create a new font cache
    pub fn new(font_path: Option<&Path>, font_size: f32) -> ScreenshotResult<Self> {
        let font_data = if let Some(path) = font_path {
            // Load from file
            std::fs::read(path).map_err(|e| {
                ScreenshotError::FontLoadError(format!("Failed to read font file: {}", e))
            })?
        } else {
            // Use embedded default font (JetBrains Mono)
            DEFAULT_FONT.to_vec()
        };

        let regular = SwashFont::new(font_data)?;

        Ok(Self {
            regular,
            emoji_font: None,               // Lazy-loaded when needed
            cjk_fonts: Vec::new(),          // Lazy-loaded when needed
            cjk_font_cache: HashMap::new(), // Cache for character-to-font mapping
            cache: HashMap::new(),
            font_size,
            cell_dimensions: None,
            font_metrics: None,
            scaler: ScaleContext::new(),
        })
    }

    /// Try to load a system emoji font (lazy-loaded)
    fn try_load_emoji_font(&mut self) {
        if self.emoji_font.is_some() {
            return; // Already loaded
        }

        // Common emoji/unicode font paths across platforms
        let emoji_paths = [
            // macOS - Apple Color Emoji (primary emoji font)
            "/System/Library/Fonts/Apple Color Emoji.ttc",
            // macOS - Arial Unicode has excellent coverage (23MB font with lots of glyphs)
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/Library/Fonts/Arial Unicode.ttf",
            // macOS - Supplemental folder has good fonts for unicode
            "/System/Library/Fonts/Supplemental/DejaVu Sans.ttf",
            "/System/Library/Fonts/Supplemental/DejaVuSans.ttf",
            // macOS - CJK and symbol fonts
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
            "/System/Library/Fonts/CJKSymbolsFallback.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            // macOS - Apple Symbols has many symbols
            "/System/Library/Fonts/Apple Symbols.ttf",
            // Linux - Noto Color Emoji (best for emoji)
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/truetype/noto-color-emoji/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto-color-emoji/NotoColorEmoji.ttf",
            // Linux - Noto fonts have excellent unicode coverage
            "/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            // Windows - Segoe UI Emoji (color emoji font)
            "C:\\Windows\\Fonts\\seguiemj.ttf",
            // Windows - Segoe UI Symbol has good symbol/emoji coverage
            "C:\\Windows\\Fonts\\seguisym.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
            "C:\\Windows\\Fonts\\msgothic.ttc",
            "C:\\Windows\\Fonts\\msyh.ttc",
        ];

        for path in &emoji_paths {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = SwashFont::new(data) {
                    self.emoji_font = Some(font);
                    return;
                }
            }
        }

        // After all system fonts fail, try embedded emoji font as final fallback
        // This ensures emoji render (even if monochrome) on all platforms
        if let Ok(font) = SwashFont::new(EMOJI_FALLBACK_FONT.to_vec()) {
            self.emoji_font = Some(font);
        }

        // If even embedded font fails to load, emoji/unicode will render as tofu (□) boxes
    }

    /// Try to load system CJK fonts (lazy-loaded)
    /// Loads ALL available CJK fonts in priority order for font fallback
    fn try_load_cjk_font(&mut self) {
        if !self.cjk_fonts.is_empty() {
            return; // Already loaded
        }

        // Common CJK font paths across platforms
        // NOTE: Order matters! Fonts are tried in this order for missing glyphs.
        let cjk_paths = [
            // macOS - High quality comprehensive CJK fonts
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf", // Excellent CJK coverage
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/PingFang.ttc", // Modern Chinese font
            "/System/Library/Fonts/Hiragino Sans GB.ttc", // Chinese
            "/System/Library/Fonts/AppleSDGothicNeo.ttc", // Korean
            "/System/Library/Fonts/Hiragino Kaku Gothic ProN.ttc", // Japanese
            // macOS - Punctuation specialists
            "/System/Library/Fonts/CJKSymbolsFallback.ttc", // CJK punctuation & symbols
            // macOS - Additional system fonts with CJK support
            "/System/Library/Fonts/STHeiti Medium.ttc", // Legacy Chinese font
            "/System/Library/Fonts/AppleMyungjo.ttf",   // Korean serif
            "/System/Library/Fonts/STSong.ttf",         // Chinese serif
            // Linux - Noto CJK fonts (excellent coverage)
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto-cjk/NotoSansCJK-Regular.ttc",
            // Linux - Good general fonts with wide Unicode support
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            // Windows - CJK fonts
            "C:\\Windows\\Fonts\\msgothic.ttc", // Japanese
            "C:\\Windows\\Fonts\\msyh.ttc",     // Chinese
            "C:\\Windows\\Fonts\\malgun.ttf",   // Korean
            "C:\\Windows\\Fonts\\arial.ttf",    // Has some CJK
        ];

        // Load ALL available fonts, not just the first one
        for path in &cjk_paths {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = SwashFont::new(data) {
                    self.cjk_fonts.push(font);
                }
            }
        }

        // If no system CJK fonts found, we'll fall back to the main font's tofu boxes
    }

    /// Check if a bitmap is empty (all zeros)
    fn is_bitmap_empty(bitmap: &[u8]) -> bool {
        bitmap.iter().all(|&b| b == 0)
    }

    /// Scale a bitmap to fit within target dimensions
    /// Uses nearest-neighbor sampling for simplicity
    fn scale_bitmap(
        bitmap: &[u8],
        src_width: usize,
        src_height: usize,
        target_width: usize,
        target_height: usize,
        format: &BitmapFormat,
    ) -> Vec<u8> {
        // If already the right size, no need to scale
        if src_width == target_width && src_height == target_height {
            return bitmap.to_vec();
        }

        let bytes_per_pixel = match format {
            BitmapFormat::Grayscale => 1,
            BitmapFormat::Rgba => 4,
        };

        let mut scaled = vec![0u8; target_width * target_height * bytes_per_pixel];

        // Nearest-neighbor resampling
        for y in 0..target_height {
            for x in 0..target_width {
                // Map target coordinates to source coordinates
                let src_x = (x * src_width) / target_width;
                let src_y = (y * src_height) / target_height;

                let src_idx = (src_y * src_width + src_x) * bytes_per_pixel;
                let dst_idx = (y * target_width + x) * bytes_per_pixel;

                // Copy pixel data
                for i in 0..bytes_per_pixel {
                    if src_idx + i < bitmap.len() && dst_idx + i < scaled.len() {
                        scaled[dst_idx + i] = bitmap[src_idx + i];
                    }
                }
            }
        }

        scaled
    }

    /// Rasterize a glyph using swash
    fn rasterize_glyph(
        scaler: &mut ScaleContext,
        font: &SwashFont,
        c: char,
        font_size: f32,
        bold: bool,
    ) -> ScreenshotResult<(GlyphMetrics, Vec<u8>, BitmapFormat)> {
        let font_ref = font
            .font_ref()
            .ok_or_else(|| ScreenshotError::FontLoadError("Failed to get font ref".to_string()))?;

        // Get glyph ID for the character
        let glyph_id = match font_ref.charmap().map(c) {
            id if id > 0 => id,
            _ => {
                return Err(ScreenshotError::FontLoadError(
                    "Character not in font".to_string(),
                ));
            }
        };

        // Create scaler for this font
        let mut font_scaler = scaler.builder(font_ref).size(font_size).hint(true).build();

        // Use Render to automatically try different glyph sources in order:
        // 1. Color outline (for color vector emoji)
        // 2. Color bitmap (for bitmap color emoji fonts)
        // 3. Outline (for regular TrueType/OpenType outline fonts)
        // 4. Bitmap (for bitmap fonts)
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
            Source::Bitmap(StrikeWith::BestFit),
        ])
        .render(&mut font_scaler, glyph_id);

        let image = match image {
            Some(img) => img,
            None => {
                // No glyph (e.g., unsupported character)
                return Ok((
                    GlyphMetrics {
                        xmin: 0,
                        ymin: 0,
                        width: 0,
                        height: 0,
                        advance_width: 0.0,
                        advance_height: 0.0,
                    },
                    vec![],
                    BitmapFormat::Grayscale,
                ));
            }
        };

        // Get metrics
        let placement = image.placement;
        let glyph_advance = font_ref.glyph_metrics(&[]).advance_width(glyph_id);
        let units_per_em = font_ref.metrics(&[]).units_per_em as f32;
        let advance_width = glyph_advance * font_size / units_per_em;

        // Process the glyph image based on content type
        use swash::scale::image::Content;
        let (final_bitmap, bitmap_format) = match image.content {
            Content::Mask => {
                // Grayscale/alpha mask
                (image.data.to_vec(), BitmapFormat::Grayscale)
            }
            Content::Color => {
                // Color bitmap from swash
                // swash documentation says BGRA, but testing shows we need to swap R<->B
                // to get correct colors (R and B were swapped)
                let mut rgba = Vec::with_capacity(image.data.len());
                for chunk in image.data.chunks_exact(4) {
                    rgba.push(chunk[0]); // R (actually at position 0, not 2)
                    rgba.push(chunk[1]); // G
                    rgba.push(chunk[2]); // B (actually at position 2, not 0)
                    rgba.push(chunk[3]); // A
                }
                (rgba, BitmapFormat::Rgba)
            }
            Content::SubpixelMask => {
                // Subpixel mask - treat as regular mask, use R channel
                let grayscale: Vec<u8> = image.data.chunks_exact(3).map(|chunk| chunk[0]).collect();
                (grayscale, BitmapFormat::Grayscale)
            }
        };

        // Apply bold if requested (not used currently, but kept for API compatibility)
        let _ = bold;

        let glyph_metrics = GlyphMetrics {
            xmin: placement.left,
            ymin: -placement.top,
            width: placement.width as usize,
            height: placement.height as usize,
            advance_width,
            advance_height: 0.0,
        };

        Ok((glyph_metrics, final_bitmap, bitmap_format))
    }

    /// Get or render a glyph
    pub fn get_glyph(&mut self, c: char, bold: bool, italic: bool) -> &CachedGlyph {
        let size_key = (self.font_size * 100.0) as u32; // Convert to integer for hash key
        let key = (c, size_key, bold, italic);

        // Check if already cached
        if self.cache.contains_key(&key) {
            return self.cache.get(&key).unwrap();
        }

        // Render new glyph - try main font first
        let (metrics, bitmap, format) =
            Self::rasterize_glyph(&mut self.scaler, &self.regular, c, self.font_size, bold)
                .unwrap_or_else(|_| {
                    (
                        GlyphMetrics {
                            xmin: 0,
                            ymin: 0,
                            width: 0,
                            height: 0,
                            advance_width: 0.0,
                            advance_height: 0.0,
                        },
                        vec![],
                        BitmapFormat::Grayscale,
                    )
                });

        // Determine if we need to try fallback fonts:
        // 1. No metrics at all (advance_width == 0 AND width == 0)
        // 2. Character is emoji or CJK
        // 3. Bitmap is empty even though metrics exist (tofu/missing glyph)
        let needs_fallback = (metrics.advance_width == 0.0 && metrics.width == 0)
            || is_emoji(c)
            || is_cjk(c)
            || Self::is_bitmap_empty(&bitmap);

        let cached_glyph = if needs_fallback {
            // Determine which fallback to try based on character type
            let is_emoji_char = is_emoji(c);
            let is_cjk_char = is_cjk(c);

            // Try CJK fonts if it's a CJK character
            if is_cjk_char {
                // Lazy-load CJK fonts if not already loaded
                if self.cjk_fonts.is_empty() {
                    self.try_load_cjk_font();
                }

                // Check if we've already cached which font has this character
                if !bold && !italic {
                    if let Some(&font_idx) = self.cjk_font_cache.get(&c) {
                        if font_idx < self.cjk_fonts.len() {
                            if let Ok((cjk_metrics, cjk_bitmap, cjk_format)) = Self::rasterize_glyph(
                                &mut self.scaler,
                                &self.cjk_fonts[font_idx],
                                c,
                                self.font_size,
                                false,
                            ) {
                                if (cjk_metrics.advance_width > 0.0 || cjk_metrics.width > 0)
                                    && !Self::is_bitmap_empty(&cjk_bitmap)
                                {
                                    self.cache.insert(
                                        key,
                                        CachedGlyph {
                                            metrics: cjk_metrics,
                                            bitmap: cjk_bitmap,
                                            format: cjk_format,
                                        },
                                    );
                                    return self.cache.get(&key).unwrap();
                                }
                            }
                        }
                    }
                }

                // Try each CJK font in order until we find one with the glyph
                for font_idx in 0..self.cjk_fonts.len() {
                    if let Ok((cjk_metrics, cjk_bitmap, cjk_format)) = Self::rasterize_glyph(
                        &mut self.scaler,
                        &self.cjk_fonts[font_idx],
                        c,
                        self.font_size,
                        false,
                    ) {
                        let is_empty = Self::is_bitmap_empty(&cjk_bitmap);

                        if (cjk_metrics.advance_width > 0.0 || cjk_metrics.width > 0) && !is_empty {
                            // Cache which font has this character
                            if !bold && !italic {
                                self.cjk_font_cache.insert(c, font_idx);
                            }

                            self.cache.insert(
                                key,
                                CachedGlyph {
                                    metrics: cjk_metrics,
                                    bitmap: cjk_bitmap,
                                    format: cjk_format,
                                },
                            );
                            return self.cache.get(&key).unwrap();
                        }
                    }
                }
            }

            // Try emoji font if it's an emoji or if CJK fallback failed
            if is_emoji_char || needs_fallback {
                if self.emoji_font.is_none() {
                    self.try_load_emoji_font();
                }

                if self.emoji_font.is_some() {
                    // Temporarily take ownership to avoid borrow issues
                    let emoji_result = {
                        let emoji_font = self.emoji_font.as_ref().unwrap();
                        Self::rasterize_glyph(
                            &mut self.scaler,
                            emoji_font,
                            c,
                            self.font_size,
                            false,
                        )
                    };

                    if let Ok((emoji_metrics, emoji_bitmap, emoji_format)) = emoji_result {
                        if (emoji_metrics.advance_width > 0.0 || emoji_metrics.width > 0)
                            && !Self::is_bitmap_empty(&emoji_bitmap)
                        {
                            // Scale emoji to fit cell height if needed
                            let target_height = self.font_size.ceil() as usize;
                            let (scaled_bitmap, scaled_metrics) = if emoji_metrics.height
                                > target_height
                            {
                                let scale = target_height as f32 / emoji_metrics.height as f32;
                                let target_width =
                                    (emoji_metrics.width as f32 * scale).ceil() as usize;

                                let scaled = Self::scale_bitmap(
                                    &emoji_bitmap,
                                    emoji_metrics.width,
                                    emoji_metrics.height,
                                    target_width,
                                    target_height,
                                    &emoji_format,
                                );

                                let mut scaled_metrics = emoji_metrics.clone();
                                scaled_metrics.width = target_width;
                                scaled_metrics.height = target_height;
                                scaled_metrics.ymin = (emoji_metrics.ymin as f32 * scale) as i32;
                                scaled_metrics.xmin = (emoji_metrics.xmin as f32 * scale) as i32;

                                (scaled, scaled_metrics)
                            } else {
                                (emoji_bitmap, emoji_metrics)
                            };

                            self.cache.insert(
                                key,
                                CachedGlyph {
                                    metrics: scaled_metrics,
                                    bitmap: scaled_bitmap,
                                    format: emoji_format,
                                },
                            );
                            return self.cache.get(&key).unwrap();
                        }
                    }
                }
            }

            // Fallback failed, use main font result
            CachedGlyph {
                metrics,
                bitmap,
                format,
            }
        } else {
            // Main font has the glyph
            CachedGlyph {
                metrics,
                bitmap,
                format,
            }
        };

        // Insert and return reference
        self.cache.insert(key, cached_glyph);
        self.cache.get(&key).unwrap()
    }

    /// Rasterize a glyph by glyph ID (from shaped text)
    fn rasterize_glyph_by_id(
        scaler: &mut ScaleContext,
        font: &SwashFont,
        glyph_id: u32,
        font_size: f32,
        bold: bool,
    ) -> ScreenshotResult<(GlyphMetrics, Vec<u8>, BitmapFormat)> {
        let font_ref = font
            .font_ref()
            .ok_or_else(|| ScreenshotError::FontLoadError("Failed to get font ref".to_string()))?;

        let glyph_id = glyph_id as u16; // swash uses u16 for glyph IDs

        // Create scaler for this font
        let mut font_scaler = scaler.builder(font_ref).size(font_size).hint(true).build();

        // Use Render to automatically try different glyph sources in order
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
            Source::Bitmap(StrikeWith::BestFit),
        ])
        .render(&mut font_scaler, glyph_id);

        let image = match image {
            Some(img) => img,
            None => {
                return Ok((
                    GlyphMetrics {
                        xmin: 0,
                        ymin: 0,
                        width: 0,
                        height: 0,
                        advance_width: 0.0,
                        advance_height: 0.0,
                    },
                    vec![],
                    BitmapFormat::Grayscale,
                ));
            }
        };

        // Get metrics
        let placement = image.placement;
        let glyph_advance = font_ref.glyph_metrics(&[]).advance_width(glyph_id);
        let units_per_em = font_ref.metrics(&[]).units_per_em as f32;
        let advance_width = glyph_advance * font_size / units_per_em;

        // Process the glyph image
        use swash::scale::image::Content;
        let (final_bitmap, bitmap_format) = match image.content {
            Content::Mask => (image.data.to_vec(), BitmapFormat::Grayscale),
            Content::Color => {
                // Same as above - no BGRA conversion needed
                let mut rgba = Vec::with_capacity(image.data.len());
                for chunk in image.data.chunks_exact(4) {
                    rgba.push(chunk[0]); // R
                    rgba.push(chunk[1]); // G
                    rgba.push(chunk[2]); // B
                    rgba.push(chunk[3]); // A
                }
                (rgba, BitmapFormat::Rgba)
            }
            Content::SubpixelMask => {
                let grayscale: Vec<u8> = image.data.chunks_exact(3).map(|chunk| chunk[0]).collect();
                (grayscale, BitmapFormat::Grayscale)
            }
        };

        let _ = bold;

        let glyph_metrics = GlyphMetrics {
            xmin: placement.left,
            ymin: -placement.top,
            width: placement.width as usize,
            height: placement.height as usize,
            advance_width,
            advance_height: 0.0,
        };

        Ok((glyph_metrics, final_bitmap, bitmap_format))
    }

    /// Get a glyph by ID for shaped text rendering
    pub fn get_glyph_by_id(
        &mut self,
        glyph_id: u32,
        font_type: super::shaper::FontType,
        bold: bool,
    ) -> Option<CachedGlyph> {
        use super::shaper::FontType;

        // Ensure fonts are loaded
        match font_type {
            FontType::Emoji => {
                if self.emoji_font.is_none() {
                    self.try_load_emoji_font();
                }
            }
            FontType::Cjk => {
                if self.cjk_fonts.is_empty() {
                    self.try_load_cjk_font();
                }
            }
            _ => {}
        }

        // Get the font to use
        let font = match font_type {
            FontType::Regular => &self.regular,
            FontType::Emoji => self.emoji_font.as_ref().unwrap_or(&self.regular),
            FontType::Cjk => self.cjk_fonts.first().unwrap_or(&self.regular),
        };

        let (mut metrics, bitmap, format) =
            Self::rasterize_glyph_by_id(&mut self.scaler, font, glyph_id, self.font_size, bold)
                .ok()?;

        // For emoji, scale if needed
        let target_height = self.font_size.ceil() as usize;
        let (final_bitmap, final_metrics) =
            if matches!(font_type, FontType::Emoji) && metrics.height > target_height {
                let scale = target_height as f32 / metrics.height as f32;
                let target_width = (metrics.width as f32 * scale).ceil() as usize;

                let scaled = Self::scale_bitmap(
                    &bitmap,
                    metrics.width,
                    metrics.height,
                    target_width,
                    target_height,
                    &format,
                );

                metrics.width = target_width;
                metrics.height = target_height;
                metrics.ymin = (metrics.ymin as f32 * scale) as i32;
                metrics.xmin = (metrics.xmin as f32 * scale) as i32;

                (scaled, metrics)
            } else {
                (bitmap, metrics)
            };

        Some(CachedGlyph {
            metrics: final_metrics,
            bitmap: final_bitmap,
            format,
        })
    }

    /// Get cell dimensions (width, height) in pixels
    pub fn cell_dimensions(&mut self) -> (u32, u32) {
        if let Some(dims) = self.cell_dimensions {
            return dims;
        }

        // Measure a typical monospace character to get dimensions
        let glyph = self.get_glyph('M', false, false);
        let advance_width = glyph.metrics.advance_width;
        let width = advance_width.ceil() as u32;

        // Use actual font line height (ascent - descent) instead of just font size
        // This ensures glyphs have enough vertical space and don't overlap
        let (ascent, descent, _) = self.get_font_metrics();
        let height = (ascent - descent) as u32; // descent is negative, so this adds them

        self.cell_dimensions = Some((width, height));
        (width, height)
    }

    /// Get cached font metrics (ascender, descender, height)
    fn get_font_metrics(&mut self) -> (i32, i32, i32) {
        if let Some(metrics) = self.font_metrics {
            return metrics;
        }

        // Get metrics from the main font
        if let Some(font_ref) = self.regular.font_ref() {
            let swash_metrics = font_ref.metrics(&[]);

            // Scale metrics to font size
            let scale = self.font_size / swash_metrics.units_per_em as f32;
            let ascender = (swash_metrics.ascent * scale) as i32;
            // Note: swash returns descent as positive, but we need it negative (distance below baseline)
            let descender = -(swash_metrics.descent * scale) as i32;
            let height = (swash_metrics.ascent - swash_metrics.descent) * scale;
            let height = height as i32;

            self.font_metrics = Some((ascender, descender, height));
            (ascender, descender, height)
        } else {
            // Fallback values
            let ascender = (self.font_size * 0.8) as i32;
            let descender = -(self.font_size * 0.2) as i32;
            let height = self.font_size as i32;
            self.font_metrics = Some((ascender, descender, height));
            (ascender, descender, height)
        }
    }

    /// Get the ascent (distance from baseline to top of tallest glyphs)
    pub fn ascent(&mut self) -> i32 {
        let (ascender, _, _) = self.get_font_metrics();
        ascender
    }

    /// Get the descent (distance from baseline to bottom of descenders)
    pub fn descent(&mut self) -> i32 {
        let (_, descender, _) = self.get_font_metrics();
        descender // Note: this is negative
    }

    /// Get the regular font data for use with text shaper
    pub fn regular_font_data(&self) -> Arc<Vec<u8>> {
        self.regular.font_data().clone()
    }

    /// Get the emoji font data for use with text shaper (if loaded)
    pub fn emoji_font_data(&mut self) -> Option<Arc<Vec<u8>>> {
        // Lazy load emoji font if not already loaded
        if self.emoji_font.is_none() {
            self.try_load_emoji_font();
        }
        self.emoji_font.as_ref().map(|f| f.font_data().clone())
    }

    /// Get the CJK font data for use with text shaper (if loaded)
    /// Returns the first (primary) CJK font data
    pub fn cjk_font_data(&mut self) -> Option<Arc<Vec<u8>>> {
        // Lazy load CJK fonts if not already loaded
        if self.cjk_fonts.is_empty() {
            self.try_load_cjk_font();
        }
        self.cjk_fonts.first().map(|f| f.font_data().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_cache_creation() {
        let result = FontCache::new(None, 14.0);
        // Don't assert here as system fonts may not be available in all test environments
        if let Ok(mut cache) = result {
            let dims = cache.cell_dimensions();
            println!("Cell dimensions: {:?}", dims);
            assert!(dims.0 > 0);
            assert!(dims.1 > 0);
        }
    }

    #[test]
    fn test_glyph_caching() {
        if let Ok(mut cache) = FontCache::new(None, 14.0) {
            // Get the same glyph twice - second time should be from cache
            let width1 = cache.get_glyph('A', false, false).metrics.width;
            let width2 = cache.get_glyph('A', false, false).metrics.width;

            // They should be the same (cached)
            assert_eq!(width1, width2);
        }
    }

    #[test]
    fn test_advance_width_calculation() {
        if let Ok(mut cache) = FontCache::new(None, 14.0) {
            let glyph = cache.get_glyph('M', false, false);
            println!("Glyph 'M' metrics:");
            println!("  advance_width: {}", glyph.metrics.advance_width);
            println!("  width: {}", glyph.metrics.width);
            println!("  height: {}", glyph.metrics.height);

            // For a 14px monospace font, advance width should be around 8-10 pixels
            assert!(
                glyph.metrics.advance_width > 5.0,
                "Advance width {} is too small",
                glyph.metrics.advance_width
            );
            assert!(
                glyph.metrics.advance_width < 20.0,
                "Advance width {} is too large",
                glyph.metrics.advance_width
            );
        }
    }

    #[test]
    fn test_font_metrics() {
        if let Ok(mut cache) = FontCache::new(None, 14.0) {
            let ascent = cache.ascent();
            let descent = cache.descent();
            let (width, height) = cache.cell_dimensions();

            println!("Font metrics at 14px:");
            println!("  ascent: {}", ascent);
            println!("  descent: {}", descent);
            println!("  line height (ascent - descent): {}", ascent - descent);
            println!("  cell_width: {}", width);
            println!("  cell_height: {}", height);

            // Ascent should be positive
            assert!(ascent > 0, "Ascent {} should be positive", ascent);
            // Descent should be negative
            assert!(descent < 0, "Descent {} should be negative", descent);
            // Line height should be reasonable
            let line_height = ascent - descent;
            assert!(line_height > 10, "Line height {} is too small", line_height);
            assert!(line_height < 30, "Line height {} is too large", line_height);
        }
    }
}
