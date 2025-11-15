use image::{Rgba, RgbaImage};

/// Check if a character is an emoji (not CJK text)
pub fn is_emoji(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        // Emoji ranges
        0x1F000..=0x1FFFF | // All emoji and symbols in Supplementary Multilingual Plane
        0x2600..=0x26FF |   // Miscellaneous Symbols
        0x2700..=0x27BF |   // Dingbats
        0xFE00..=0xFE0F     // Variation Selectors (for emoji presentation)
    )
}

/// Check if a character is CJK (Chinese, Japanese, Korean)
///
/// This is the canonical implementation used throughout the screenshot module.
/// It includes all CJK Unicode ranges for proper font fallback.
pub fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        // CJK ideographs (Chinese, Japanese, Korean)
        0x4E00..=0x9FFF |   // CJK Unified Ideographs
        0x3400..=0x4DBF |   // CJK Extension A
        0x20000..=0x2CEAF | // CJK Extensions B-E
        0xF900..=0xFAFF |   // CJK Compatibility Ideographs
        0x2F800..=0x2FA1F | // CJK Compatibility Ideographs Supplement
        // CJK Symbols and Punctuation
        0x3000..=0x303F |   // CJK Symbols and Punctuation (includes ideographic space, punctuation)
        // Hiragana and Katakana
        0x3040..=0x309F |   // Hiragana
        0x30A0..=0x30FF |   // Katakana
        0x31F0..=0x31FF |   // Katakana Phonetic Extensions
        // Hangul (Korean)
        0xAC00..=0xD7AF |   // Hangul Syllables
        0x1100..=0x11FF |   // Hangul Jamo
        0x3130..=0x318F |   // Hangul Compatibility Jamo
        0xA960..=0xA97F |   // Hangul Jamo Extended-A
        0xD7B0..=0xD7FF |   // Hangul Jamo Extended-B
        // Fullwidth Forms (fullwidth ASCII variants and halfwidth Katakana)
        0xFF00..=0xFFEF |   // Halfwidth and Fullwidth Forms (includes ！ U+FF01, etc.)
        // Enclosed CJK Letters and Months
        0x3200..=0x32FF |   // Enclosed CJK Letters and Months
        0x3300..=0x33FF |   // CJK Compatibility
        // Additional CJK Radicals
        0x2E80..=0x2EFF |   // CJK Radicals Supplement
        0x2F00..=0x2FDF     // Kangxi Radicals
    )
}

/// Blend a grayscale glyph pixel onto the image using alpha blending
///
/// # Arguments
/// * `image` - The target image to render into
/// * `px` - Pixel X coordinate
/// * `py` - Pixel Y coordinate
/// * `fg` - Foreground color (R, G, B)
/// * `alpha` - Alpha value (0-255)
/// * `canvas_width` - Canvas width for bounds checking
/// * `canvas_height` - Canvas height for bounds checking
#[inline]
pub fn blend_grayscale_pixel(
    image: &mut RgbaImage,
    px: u32,
    py: u32,
    fg: (u8, u8, u8),
    alpha: u8,
    canvas_width: u32,
    canvas_height: u32,
) {
    if alpha == 0 || px >= canvas_width || py >= canvas_height {
        return;
    }

    let existing = image.get_pixel(px, py);
    let alpha_f = alpha as f32 / 255.0;
    let inv_alpha = 1.0 - alpha_f;

    let r = (fg.0 as f32 * alpha_f + existing[0] as f32 * inv_alpha) as u8;
    let g = (fg.1 as f32 * alpha_f + existing[1] as f32 * inv_alpha) as u8;
    let b = (fg.2 as f32 * alpha_f + existing[2] as f32 * inv_alpha) as u8;

    image.put_pixel(px, py, Rgba([r, g, b, 255]));
}

/// Blend an RGBA pixel onto the image using alpha blending
///
/// # Arguments
/// * `image` - The target image to render into
/// * `px` - Pixel X coordinate
/// * `py` - Pixel Y coordinate
/// * `color` - RGB color (R, G, B)
/// * `alpha` - Alpha value (0-255)
/// * `canvas_width` - Canvas width for bounds checking
/// * `canvas_height` - Canvas height for bounds checking
#[inline]
pub fn blend_rgba_pixel(
    image: &mut RgbaImage,
    px: u32,
    py: u32,
    color: (u8, u8, u8),
    alpha: u8,
    canvas_width: u32,
    canvas_height: u32,
) {
    if alpha == 0 || px >= canvas_width || py >= canvas_height {
        return;
    }

    let (r, g, b) = color;

    if alpha == 255 {
        // Fully opaque - just copy the pixel
        image.put_pixel(px, py, Rgba([r, g, b, 255]));
    } else {
        // Get existing pixel for alpha blending
        let existing = image.get_pixel(px, py);
        let alpha_f = alpha as f32 / 255.0;
        let inv_alpha = 1.0 - alpha_f;

        let blended_r = (r as f32 * alpha_f + existing[0] as f32 * inv_alpha) as u8;
        let blended_g = (g as f32 * alpha_f + existing[1] as f32 * inv_alpha) as u8;
        let blended_b = (b as f32 * alpha_f + existing[2] as f32 * inv_alpha) as u8;

        image.put_pixel(px, py, Rgba([blended_r, blended_g, blended_b, 255]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_emoji() {
        // Should detect emoji
        assert!(is_emoji('🚀'));
        assert!(is_emoji('❤'));
        assert!(is_emoji('☕'));

        // Should not detect regular text
        assert!(!is_emoji('A'));
        assert!(!is_emoji('中'));
        assert!(!is_emoji(' '));
    }

    #[test]
    fn test_is_cjk() {
        // Chinese characters
        assert!(is_cjk('中'));
        assert!(is_cjk('国'));

        // Japanese characters
        assert!(is_cjk('日'));
        assert!(is_cjk('本'));
        assert!(is_cjk('あ')); // Hiragana
        assert!(is_cjk('ア')); // Katakana

        // Korean characters
        assert!(is_cjk('한'));
        assert!(is_cjk('국'));

        // CJK punctuation
        assert!(is_cjk('、')); // U+3001 - Ideographic comma
        assert!(is_cjk('。')); // U+3002 - Ideographic full stop
        assert!(is_cjk('！')); // U+FF01 - Fullwidth exclamation mark
        assert!(is_cjk('？')); // U+FF1F - Fullwidth question mark

        // Should not detect regular text
        assert!(!is_cjk('A'));
        assert!(!is_cjk('z'));
        assert!(!is_cjk('0'));
        assert!(!is_cjk(' '));

        // Should not detect emoji
        assert!(!is_cjk('🚀'));
    }

    #[test]
    fn test_cjk_comprehensive_ranges() {
        // Test each major CJK range has at least one character detected
        assert!(is_cjk('\u{4E00}')); // CJK Unified Ideographs start
        assert!(is_cjk('\u{3400}')); // CJK Extension A start
        assert!(is_cjk('\u{3040}')); // Hiragana start
        assert!(is_cjk('\u{30A0}')); // Katakana start
        assert!(is_cjk('\u{AC00}')); // Hangul Syllables start
        assert!(is_cjk('\u{FF01}')); // Fullwidth Forms (！)
        assert!(is_cjk('\u{3200}')); // Enclosed CJK Letters
        assert!(is_cjk('\u{2E80}')); // CJK Radicals Supplement
    }

    #[test]
    fn test_blend_grayscale_pixel() {
        let mut image = RgbaImage::new(10, 10);
        // Fill with white background
        for y in 0..10 {
            for x in 0..10 {
                image.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }

        // Blend black with 50% alpha (128/255 ≈ 0.5)
        // Expected: 0 * 0.5 + 255 * 0.5 = 127.5 ≈ 127
        blend_grayscale_pixel(&mut image, 5, 5, (0, 0, 0), 128, 10, 10);

        let pixel = image.get_pixel(5, 5);
        // Should be approximately gray (with 50% alpha, we get about 127-128)
        assert!(pixel[0] >= 126 && pixel[0] <= 129, "Got {}", pixel[0]);
        assert!(pixel[1] >= 126 && pixel[1] <= 129, "Got {}", pixel[1]);
        assert!(pixel[2] >= 126 && pixel[2] <= 129, "Got {}", pixel[2]);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn test_blend_rgba_pixel() {
        let mut image = RgbaImage::new(10, 10);
        // Fill with white background
        for y in 0..10 {
            for x in 0..10 {
                image.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }

        // Blend red with full alpha
        blend_rgba_pixel(&mut image, 3, 3, (255, 0, 0), 255, 10, 10);
        let pixel = image.get_pixel(3, 3);
        assert_eq!(*pixel, Rgba([255, 0, 0, 255]));

        // Blend blue with 50% alpha (128/255 ≈ 0.5)
        // R: 0 * 0.5 + 255 * 0.5 = 127.5 ≈ 127
        // G: 0 * 0.5 + 255 * 0.5 = 127.5 ≈ 127
        // B: 255 * 0.5 + 255 * 0.5 = 255
        blend_rgba_pixel(&mut image, 5, 5, (0, 0, 255), 128, 10, 10);
        let pixel = image.get_pixel(5, 5);
        // Should be approximately light blue
        assert!(pixel[0] >= 126 && pixel[0] <= 129, "R: Got {}", pixel[0]);
        assert!(pixel[1] >= 126 && pixel[1] <= 129, "G: Got {}", pixel[1]);
        assert!(pixel[2] >= 254, "B: Got {}", pixel[2]); // Blue should be close to 255
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn test_blend_out_of_bounds() {
        let mut image = RgbaImage::new(10, 10);

        // Should not panic on out-of-bounds coordinates
        blend_grayscale_pixel(&mut image, 100, 100, (0, 0, 0), 255, 10, 10);
        blend_rgba_pixel(&mut image, 100, 100, (255, 0, 0), 255, 10, 10);
    }

    #[test]
    fn test_blend_zero_alpha() {
        let mut image = RgbaImage::new(10, 10);
        image.put_pixel(5, 5, Rgba([100, 100, 100, 255]));

        // Should not change pixel with zero alpha
        blend_grayscale_pixel(&mut image, 5, 5, (255, 0, 0), 0, 10, 10);
        let pixel = image.get_pixel(5, 5);
        assert_eq!(*pixel, Rgba([100, 100, 100, 255]));
    }
}
