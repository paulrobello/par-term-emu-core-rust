//! Color manipulation and conversion utilities
//!
//! This module provides advanced color manipulation functions including:
//! - Minimum contrast adjustment (iTerm2-compatible)
//! - Perceived brightness calculation (NTSC formula)
//! - Color space conversions (RGB, HSL)
//! - WCAG contrast ratio calculations

use crate::color::Color;

/// Calculate perceived brightness of an RGB color using NTSC formula.
///
/// The NTSC formula weights color components based on human perception:
/// - Red: 30%
/// - Green: 59%
/// - Blue: 11%
///
/// # Arguments
///
/// * `r` - Red component (0-255)
/// * `g` - Green component (0-255)
/// * `b` - Blue component (0-255)
///
/// # Returns
///
/// Perceived brightness value (0.0-1.0)
#[inline]
pub fn perceived_brightness_rgb(r: u8, g: u8, b: u8) -> f64 {
    const RED_WEIGHT: f64 = 0.30;
    const GREEN_WEIGHT: f64 = 0.59;
    const BLUE_WEIGHT: f64 = 0.11;

    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    r * RED_WEIGHT + g * GREEN_WEIGHT + b * BLUE_WEIGHT
}

/// Adjust color components to have a specific target brightness.
///
/// Uses parametric interpolation between the original color and either
/// white (1,1,1) or black (0,0,0) to achieve the target brightness while
/// preserving the color's hue as much as possible.
///
/// # Arguments
///
/// * `r` - Red component (0.0-1.0)
/// * `g` - Green component (0.0-1.0)
/// * `b` - Blue component (0.0-1.0)
/// * `target_brightness` - Target perceived brightness (0.0-1.0)
///
/// # Returns
///
/// Adjusted (r, g, b) components (0.0-1.0)
fn adjust_brightness_normalized(r: f64, g: f64, b: f64, target_brightness: f64) -> (f64, f64, f64) {
    let current_brightness = r * 0.30 + g * 0.59 + b * 0.11;

    // Determine extreme point: white (1,1,1) if we need to brighten, black (0,0,0) if we need to dim
    let extreme = if current_brightness < target_brightness {
        1.0
    } else {
        0.0
    };

    // Calculate parametric interpolation factor
    // p = (target - current) / (extreme - current)
    let denominator = (extreme - r) * 0.30 + (extreme - g) * 0.59 + (extreme - b) * 0.11;
    let p = if denominator.abs() < 1e-10 {
        0.0
    } else {
        (target_brightness - current_brightness) / denominator
    };

    // Clamp p to valid range [0, 1]
    let p = p.clamp(0.0, 1.0);

    // Apply parametric interpolation: result = p * extreme + (1 - p) * original
    let new_r = p * extreme + (1.0 - p) * r;
    let new_g = p * extreme + (1.0 - p) * g;
    let new_b = p * extreme + (1.0 - p) * b;

    (new_r, new_g, new_b)
}

/// Adjust foreground color to maintain minimum contrast against background.
///
/// Implements iTerm2's minimum contrast algorithm:
/// 1. Calculate brightness difference between fg and bg
/// 2. If difference < minimum_contrast, adjust fg brightness
/// 3. Try moving fg away from bg first
/// 4. If that would exceed bounds, try opposite direction
/// 5. Choose direction that provides better contrast
///
/// # Arguments
///
/// * `fg` - Foreground color (r, g, b)
/// * `bg` - Background color (r, g, b)
/// * `minimum_contrast` - Minimum required brightness difference (0.0-1.0)
///
/// # Returns
///
/// Adjusted foreground (r, g, b)
///
/// # Examples
///
/// ```
/// use par_term_emu_core_rust::color_utils::adjust_contrast_rgb;
///
/// // Dark gray text on black background - will be lightened
/// let fg = (64, 64, 64);
/// let bg = (0, 0, 0);
/// let adjusted = adjust_contrast_rgb(fg, bg, 0.5);
/// assert!(adjusted.0 > fg.0);
/// ```
pub fn adjust_contrast_rgb(
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    minimum_contrast: f64,
) -> (u8, u8, u8) {
    // Convert to normalized 0.0-1.0 range
    let fg_r = fg.0 as f64 / 255.0;
    let fg_g = fg.1 as f64 / 255.0;
    let fg_b = fg.2 as f64 / 255.0;

    let bg_r = bg.0 as f64 / 255.0;
    let bg_g = bg.1 as f64 / 255.0;
    let bg_b = bg.2 as f64 / 255.0;

    let fg_brightness = fg_r * 0.30 + fg_g * 0.59 + fg_b * 0.11;
    let bg_brightness = bg_r * 0.30 + bg_g * 0.59 + bg_b * 0.11;
    let brightness_diff = (fg_brightness - bg_brightness).abs();

    // If contrast is already sufficient, return original color
    if brightness_diff >= minimum_contrast {
        return fg;
    }

    // Calculate error amount we need to correct
    let error = (brightness_diff - minimum_contrast).abs();
    let mut target_brightness = fg_brightness;

    if fg_brightness < bg_brightness {
        // Foreground is darker than background
        // Try to make it darker (increase contrast)
        target_brightness -= error;

        // If that would make it too dark (< 0), try the opposite direction
        if target_brightness < 0.0 {
            // Alternative: make it brighter than background
            let alternative = bg_brightness + minimum_contrast;
            let base_contrast = bg_brightness; // Contrast if we go to 0
            let alt_contrast = (alternative.min(1.0) - bg_brightness).abs();

            // Choose direction that gives better contrast
            if alt_contrast > base_contrast {
                target_brightness = alternative;
            }
        }
    } else {
        // Foreground is brighter than background
        // Try to make it brighter (increase contrast)
        target_brightness += error;

        // If that would make it too bright (> 1), try the opposite direction
        if target_brightness > 1.0 {
            // Alternative: make it darker than background
            let alternative = bg_brightness - minimum_contrast;
            let base_contrast = 1.0 - bg_brightness; // Contrast if we go to 1
            let alt_contrast = (bg_brightness - alternative.max(0.0)).abs();

            // Choose direction that gives better contrast
            if alt_contrast > base_contrast {
                target_brightness = alternative;
            }
        }
    }

    // Clamp target brightness to valid range
    target_brightness = target_brightness.clamp(0.0, 1.0);

    // Adjust color to achieve target brightness
    let (new_r, new_g, new_b) = adjust_brightness_normalized(fg_r, fg_g, fg_b, target_brightness);

    // Convert back to 8-bit RGB
    (
        (new_r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (new_g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (new_b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Extended color utilities
impl Color {
    /// Convert color to hex string
    pub fn to_hex(&self) -> String {
        let (r, g, b) = self.to_rgb();
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    }

    /// Create color from hex string
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }

        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

        Some(Color::Rgb(r, g, b))
    }

    /// Convert to nearest 256-color palette index
    pub fn to_ansi_256(&self) -> u8 {
        let (r, g, b) = self.to_rgb();

        // Check if it's a grayscale color
        if r == g && g == b {
            if r < 8 {
                return 16; // Black
            }
            if r > 248 {
                return 231; // White
            }
            // Grayscale ramp (232-255)
            return 232 + ((r - 8) / 10);
        }

        // Convert to 6x6x6 color cube (16-231)
        let r_idx = (r as f32 / 255.0 * 5.0).round() as u8;
        let g_idx = (g as f32 / 255.0 * 5.0).round() as u8;
        let b_idx = (b as f32 / 255.0 * 5.0).round() as u8;

        16 + 36 * r_idx + 6 * g_idx + b_idx
    }

    /// Lighten color by amount (0.0 to 1.0)
    pub fn lighten(&self, amount: f32) -> Self {
        let (r, g, b) = self.to_rgb();
        let amount = amount.clamp(0.0, 1.0);

        let r = (r as f32 + (255.0 - r as f32) * amount).round() as u8;
        let g = (g as f32 + (255.0 - g as f32) * amount).round() as u8;
        let b = (b as f32 + (255.0 - b as f32) * amount).round() as u8;

        Color::Rgb(r, g, b)
    }

    /// Darken color by amount (0.0 to 1.0)
    pub fn darken(&self, amount: f32) -> Self {
        let (r, g, b) = self.to_rgb();
        let amount = amount.clamp(0.0, 1.0);

        let r = (r as f32 * (1.0 - amount)).round() as u8;
        let g = (g as f32 * (1.0 - amount)).round() as u8;
        let b = (b as f32 * (1.0 - amount)).round() as u8;

        Color::Rgb(r, g, b)
    }

    /// Calculate relative luminance (WCAG formula)
    pub fn luminance(&self) -> f32 {
        let (r, g, b) = self.to_rgb();

        let r = (r as f32 / 255.0).powf(2.2);
        let g = (g as f32 / 255.0).powf(2.2);
        let b = (b as f32 / 255.0).powf(2.2);

        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Check if color is dark (luminance < 0.5)
    pub fn is_dark(&self) -> bool {
        self.luminance() < 0.5
    }

    /// Calculate WCAG contrast ratio with another color
    pub fn contrast_ratio(&self, other: &Color) -> f32 {
        let l1 = self.luminance();
        let l2 = other.luminance();

        let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };

        (lighter + 0.05) / (darker + 0.05)
    }

    /// Check if contrast ratio meets WCAG AA standard (4.5:1 for normal text)
    pub fn meets_wcag_aa(&self, other: &Color) -> bool {
        self.contrast_ratio(other) >= 4.5
    }

    /// Check if contrast ratio meets WCAG AAA standard (7:1 for normal text)
    pub fn meets_wcag_aaa(&self, other: &Color) -> bool {
        self.contrast_ratio(other) >= 7.0
    }

    /// Mix two colors with given ratio (0.0 = all self, 1.0 = all other)
    pub fn mix(&self, other: &Color, ratio: f32) -> Self {
        let (r1, g1, b1) = self.to_rgb();
        let (r2, g2, b2) = other.to_rgb();
        let ratio = ratio.clamp(0.0, 1.0);

        let r = (r1 as f32 * (1.0 - ratio) + r2 as f32 * ratio).round() as u8;
        let g = (g1 as f32 * (1.0 - ratio) + g2 as f32 * ratio).round() as u8;
        let b = (b1 as f32 * (1.0 - ratio) + b2 as f32 * ratio).round() as u8;

        Color::Rgb(r, g, b)
    }

    /// Convert to HSL (Hue, Saturation, Lightness)
    pub fn to_hsl(&self) -> (f32, f32, f32) {
        let (r, g, b) = self.to_rgb();
        let r = r as f32 / 255.0;
        let g = g as f32 / 255.0;
        let b = b as f32 / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;

        let l = (max + min) / 2.0;

        if delta == 0.0 {
            return (0.0, 0.0, l);
        }

        let s = if l < 0.5 {
            delta / (max + min)
        } else {
            delta / (2.0 - max - min)
        };

        let h = if max == r {
            ((g - b) / delta + if g < b { 6.0 } else { 0.0 }) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };

        (h * 360.0, s * 100.0, l * 100.0)
    }

    /// Create color from HSL
    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let h = (h % 360.0) / 360.0;
        let s = (s / 100.0).clamp(0.0, 1.0);
        let l = (l / 100.0).clamp(0.0, 1.0);

        if s == 0.0 {
            let gray = (l * 255.0).round() as u8;
            return Color::Rgb(gray, gray, gray);
        }

        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;

        let hue_to_rgb = |p: f32, q: f32, t: f32| -> f32 {
            let t = if t < 0.0 {
                t + 1.0
            } else if t > 1.0 {
                t - 1.0
            } else {
                t
            };

            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 1.0 / 2.0 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };

        let r = (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0).round() as u8;
        let g = (hue_to_rgb(p, q, h) * 255.0).round() as u8;
        let b = (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0).round() as u8;

        Color::Rgb(r, g, b)
    }

    /// Adjust saturation (-100 to 100, 0 = no change)
    pub fn adjust_saturation(&self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        let new_s = (s + amount).clamp(0.0, 100.0);
        Self::from_hsl(h, new_s, l)
    }

    /// Adjust hue (degrees, wraps around)
    pub fn adjust_hue(&self, degrees: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        let new_h = (h + degrees) % 360.0;
        Self::from_hsl(new_h, s, l)
    }

    /// Get complementary color (opposite on color wheel)
    pub fn complementary(&self) -> Self {
        self.adjust_hue(180.0)
    }

    /// Calculate perceived brightness using NTSC formula (iTerm2-compatible).
    ///
    /// Uses the same weights as iTerm2:
    /// - Red: 30%
    /// - Green: 59%
    /// - Blue: 11%
    ///
    /// Returns brightness in range 0.0-1.0
    pub fn perceived_brightness(&self) -> f64 {
        let (r, g, b) = self.to_rgb();
        perceived_brightness_rgb(r, g, b)
    }

    /// Adjust this color to maintain minimum contrast against a background color.
    ///
    /// Implements iTerm2's minimum contrast algorithm using NTSC perceived brightness.
    /// The algorithm preserves the color's hue while adjusting brightness.
    ///
    /// # Arguments
    ///
    /// * `background` - Background color to contrast against
    /// * `minimum_contrast` - Minimum required brightness difference (0.0-1.0)
    ///
    /// # Returns
    ///
    /// Adjusted color with sufficient contrast
    ///
    /// # Examples
    ///
    /// ```
    /// use par_term_emu_core_rust::color::Color;
    ///
    /// // Dark gray text on black background - will be lightened
    /// let fg = Color::Rgb(64, 64, 64);
    /// let bg = Color::Rgb(0, 0, 0);
    /// let adjusted = fg.with_min_contrast(&bg, 0.5);
    /// assert!(adjusted.perceived_brightness() > fg.perceived_brightness());
    /// ```
    pub fn with_min_contrast(&self, background: &Color, minimum_contrast: f64) -> Self {
        let (fg_r, fg_g, fg_b) = self.to_rgb();
        let (bg_r, bg_g, bg_b) = background.to_rgb();

        let (new_r, new_g, new_b) =
            adjust_contrast_rgb((fg_r, fg_g, fg_b), (bg_r, bg_g, bg_b), minimum_contrast);

        Color::Rgb(new_r, new_g, new_b)
    }
}

/// Convert sRGB color to Display P3 color space.
///
/// This mimics how iTerm2 renders colors on P3 displays: expand to linear sRGB,
/// convert through XYZ into the Display P3 primaries, then compress back to sRGB
/// gamma for raster output. Returning sRGB bytes lets us store to standard image
/// formats while keeping the perceived saturation boost from P3 monitors.
#[inline]
pub fn srgb_to_p3_rgb(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    // Normalize to [0,1]
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    // sRGB gamma → linear
    let lr = srgb_to_linear(r);
    let lg = srgb_to_linear(g);
    let lb = srgb_to_linear(b);

    // Linear sRGB → XYZ (D65)
    let x = 0.412_456_4 * lr + 0.357_576_1 * lg + 0.180_437_5 * lb;
    let y = 0.212_672_9 * lr + 0.715_152_2 * lg + 0.072_175 * lb;
    let z = 0.019_333_9 * lr + 0.119_192 * lg + 0.950_304_1 * lb;

    // XYZ → Display P3 linear (inverse matrix)
    let pr = 2.493_496_9 * x - 0.931_383_6 * y - 0.402_710_8 * z;
    let pg = -0.829_489 * x + 1.762_664_1 * y + 0.023_624_7 * z;
    let pb = 0.035_845_8 * x - 0.076_172_4 * y + 0.956_884_5 * z;

    // Linear → gamma again (same transfer function as sRGB)
    let pr = linear_to_srgb(pr);
    let pg = linear_to_srgb(pg);
    let pb = linear_to_srgb(pb);

    (
        (pr.clamp(0.0, 1.0) * 255.0).round() as u8,
        (pg.clamp(0.0, 1.0) * 255.0).round() as u8,
        (pb.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[inline]
fn srgb_to_linear(component: f64) -> f64 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn linear_to_srgb(component: f64) -> f64 {
    if component <= 0.003_130_8 {
        component * 12.92
    } else {
        1.055 * component.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_conversion() {
        let color = Color::Rgb(255, 128, 64);
        assert_eq!(color.to_hex(), "#FF8040");

        let parsed = Color::from_hex("#FF8040").unwrap();
        assert_eq!(parsed.to_rgb(), (255, 128, 64));
    }

    #[test]
    fn test_lighten_darken() {
        let color = Color::Rgb(100, 100, 100);
        let lighter = color.lighten(0.5);
        let darker = color.darken(0.5);

        let (r1, _, _) = lighter.to_rgb();
        let (r2, _, _) = darker.to_rgb();

        assert!(r1 > 100);
        assert!(r2 < 100);
    }

    #[test]
    fn test_is_dark() {
        assert!(Color::Rgb(0, 0, 0).is_dark());
        assert!(!Color::Rgb(255, 255, 255).is_dark());
    }

    #[test]
    fn test_contrast_ratio() {
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);

        let ratio = black.contrast_ratio(&white);
        assert!(ratio >= 20.0); // Should be 21:1
    }

    #[test]
    fn test_hsl_conversion() {
        let color = Color::Rgb(255, 0, 0); // Pure red
        let (h, s, l) = color.to_hsl();

        assert!((h - 0.0).abs() < 1.0);
        assert!((s - 100.0).abs() < 1.0);
        assert!((l - 50.0).abs() < 1.0);

        let back = Color::from_hsl(h, s, l);
        assert_eq!(back.to_rgb(), (255, 0, 0));
    }

    #[test]
    fn test_complementary() {
        let red = Color::Rgb(255, 0, 0);
        let cyan = red.complementary();

        let (r, g, b) = cyan.to_rgb();
        // Complementary of red should be cyan-ish
        assert!(g > 200 && b > 200 && r < 50);
    }

    #[test]
    fn test_perceived_brightness_black() {
        let black = Color::Rgb(0, 0, 0);
        let brightness = black.perceived_brightness();
        assert!((brightness - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_perceived_brightness_white() {
        let white = Color::Rgb(255, 255, 255);
        let brightness = white.perceived_brightness();
        assert!((brightness - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_perceived_brightness_red() {
        let red = Color::Rgb(255, 0, 0);
        let brightness = red.perceived_brightness();
        assert!((brightness - 0.30).abs() < 0.01);
    }

    #[test]
    fn test_perceived_brightness_green() {
        let green = Color::Rgb(0, 255, 0);
        let brightness = green.perceived_brightness();
        assert!((brightness - 0.59).abs() < 0.01);
    }

    #[test]
    fn test_perceived_brightness_blue() {
        let blue = Color::Rgb(0, 0, 255);
        let brightness = blue.perceived_brightness();
        assert!((brightness - 0.11).abs() < 0.01);
    }

    #[test]
    fn test_with_min_contrast_sufficient() {
        // White on black - already has good contrast
        let fg = Color::Rgb(255, 255, 255);
        let bg = Color::Rgb(0, 0, 0);
        let adjusted = fg.with_min_contrast(&bg, 0.5);
        assert_eq!(adjusted.to_rgb(), (255, 255, 255));
    }

    #[test]
    fn test_with_min_contrast_dark_on_black() {
        // Dark gray on black - should be lightened
        let fg = Color::Rgb(25, 25, 25);
        let bg = Color::Rgb(0, 0, 0);
        let adjusted = fg.with_min_contrast(&bg, 0.5);
        let (r, g, b) = adjusted.to_rgb();
        // Should be significantly lightened
        assert!(r > 100 && g > 100 && b > 100);
    }

    #[test]
    fn test_with_min_contrast_light_on_white() {
        // Light gray on white - should be darkened
        let fg = Color::Rgb(230, 230, 230);
        let bg = Color::Rgb(255, 255, 255);
        let adjusted = fg.with_min_contrast(&bg, 0.5);
        let (r, g, b) = adjusted.to_rgb();
        // Should be significantly darkened
        assert!(r < 150 && g < 150 && b < 150);
    }

    #[test]
    fn test_with_min_contrast_preserves_hue() {
        // Red text on black - should maintain reddish hue when brightened
        let fg = Color::Rgb(100, 0, 0);
        let bg = Color::Rgb(0, 0, 0);
        let adjusted = fg.with_min_contrast(&bg, 0.5);
        let (r, g, b) = adjusted.to_rgb();
        // Red should still be dominant channel
        assert!(r > g && r > b);
    }

    #[test]
    fn test_with_min_contrast_zero_minimum() {
        // No adjustment needed
        let fg = Color::Rgb(75, 100, 125);
        let bg = Color::Rgb(50, 50, 50);
        let adjusted = fg.with_min_contrast(&bg, 0.0);
        assert_eq!(adjusted.to_rgb(), (75, 100, 125));
    }

    #[test]
    fn test_adjust_contrast_rgb_standalone() {
        // Dark gray text on black background
        let fg = (64, 64, 64);
        let bg = (0, 0, 0);
        let adjusted = adjust_contrast_rgb(fg, bg, 0.5);
        assert!(adjusted.0 > fg.0);
        assert!(adjusted.1 > fg.1);
        assert!(adjusted.2 > fg.2);
    }

    #[test]
    fn test_perceived_brightness_rgb_standalone() {
        let brightness = perceived_brightness_rgb(0, 0, 0);
        assert!((brightness - 0.0).abs() < 0.01);

        let brightness = perceived_brightness_rgb(255, 255, 255);
        assert!((brightness - 1.0).abs() < 0.01);

        let brightness = perceived_brightness_rgb(255, 0, 0);
        assert!((brightness - 0.30).abs() < 0.01);
    }

    #[test]
    fn test_to_ansi_256_grayscale() {
        // Black should map to grayscale start (16)
        let black = Color::Rgb(0, 0, 0);
        assert_eq!(black.to_ansi_256(), 16);

        // White should map to near end of color cube (231)
        let white = Color::Rgb(255, 255, 255);
        assert_eq!(white.to_ansi_256(), 231);

        // Mid-gray should be in grayscale ramp (232-255)
        let gray = Color::Rgb(128, 128, 128);
        let index = gray.to_ansi_256();
        assert!(index >= 232);
    }

    #[test]
    fn test_to_ansi_256_color_cube() {
        // Pure red should map to color cube
        let red = Color::Rgb(255, 0, 0);
        let index = red.to_ansi_256();
        assert!((16..=231).contains(&index));

        // Pure green should map to color cube
        let green = Color::Rgb(0, 255, 0);
        let index = green.to_ansi_256();
        assert!((16..=231).contains(&index));

        // Pure blue should map to color cube
        let blue = Color::Rgb(0, 0, 255);
        let index = blue.to_ansi_256();
        assert!((16..=231).contains(&index));
    }

    #[test]
    fn test_to_ansi_256_specific_values() {
        // Color cube formula: 16 + 36*r + 6*g + b
        // (5, 5, 5) -> 16 + 36*5 + 6*5 + 5 = 16 + 180 + 30 + 5 = 231
        let white_cube = Color::Rgb(255, 255, 255);
        assert_eq!(white_cube.to_ansi_256(), 231);

        // (0, 0, 0) -> grayscale
        let black = Color::Rgb(0, 0, 0);
        assert_eq!(black.to_ansi_256(), 16);
    }

    #[test]
    fn test_meets_wcag_aa() {
        // Black on white should meet AA
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);
        assert!(black.meets_wcag_aa(&white));

        // Light gray on white should NOT meet AA
        let light_gray = Color::Rgb(200, 200, 200);
        assert!(!light_gray.meets_wcag_aa(&white));

        // Dark gray on black should NOT meet AA
        let dark_gray = Color::Rgb(50, 50, 50);
        assert!(!dark_gray.meets_wcag_aa(&black));
    }

    #[test]
    fn test_meets_wcag_aaa() {
        // Black on white should meet AAA
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);
        assert!(black.meets_wcag_aaa(&white));

        // Medium gray on white might not meet AAA
        let medium_gray = Color::Rgb(150, 150, 150);
        let meets_aaa = medium_gray.meets_wcag_aaa(&white);
        // AAA requires 7:1, which is stricter
        assert!(!meets_aaa);
    }

    #[test]
    fn test_mix_colors() {
        let red = Color::Rgb(255, 0, 0);
        let blue = Color::Rgb(0, 0, 255);

        // 0.0 ratio = all red
        let result = red.mix(&blue, 0.0);
        assert_eq!(result.to_rgb(), (255, 0, 0));

        // 1.0 ratio = all blue
        let result = red.mix(&blue, 1.0);
        assert_eq!(result.to_rgb(), (0, 0, 255));

        // 0.5 ratio = purple
        let result = red.mix(&blue, 0.5);
        let (r, g, b) = result.to_rgb();
        assert!(r > 100 && r < 150);
        assert_eq!(g, 0);
        assert!(b > 100 && b < 150);
    }

    #[test]
    fn test_mix_colors_clamping() {
        let red = Color::Rgb(255, 0, 0);
        let blue = Color::Rgb(0, 0, 255);

        // Ratio > 1.0 should clamp to 1.0
        let result = red.mix(&blue, 1.5);
        assert_eq!(result.to_rgb(), (0, 0, 255));

        // Ratio < 0.0 should clamp to 0.0
        let result = red.mix(&blue, -0.5);
        assert_eq!(result.to_rgb(), (255, 0, 0));
    }

    #[test]
    fn test_adjust_saturation_increase() {
        // Desaturated red
        let color = Color::Rgb(200, 150, 150);
        let (_, original_s, _) = color.to_hsl();

        // Increase saturation
        let saturated = color.adjust_saturation(20.0);
        let (_, new_s, _) = saturated.to_hsl();

        assert!(new_s > original_s);
    }

    #[test]
    fn test_adjust_saturation_decrease() {
        // Pure red
        let red = Color::Rgb(255, 0, 0);
        let (_, original_s, _) = red.to_hsl();

        // Decrease saturation
        let desaturated = red.adjust_saturation(-20.0);
        let (_, new_s, _) = desaturated.to_hsl();

        assert!(new_s < original_s);
    }

    #[test]
    fn test_adjust_saturation_clamping() {
        let color = Color::Rgb(200, 100, 100);

        // Over-saturate (should clamp to 100)
        let over = color.adjust_saturation(200.0);
        let (_, s, _) = over.to_hsl();
        assert!((s - 100.0).abs() < 1.0);

        // Under-saturate (should clamp to 0 - grayscale)
        let under = color.adjust_saturation(-200.0);
        let (_, s, _) = under.to_hsl();
        assert!(s < 1.0);
    }

    #[test]
    fn test_adjust_hue_basic() {
        let red = Color::Rgb(255, 0, 0);
        let (original_h, _, _) = red.to_hsl();

        // Shift hue by 60 degrees
        let shifted = red.adjust_hue(60.0);
        let (new_h, _, _) = shifted.to_hsl();

        let expected_h = (original_h + 60.0) % 360.0;
        assert!((new_h - expected_h).abs() < 2.0);
    }

    #[test]
    fn test_adjust_hue_wrapping() {
        let red = Color::Rgb(255, 0, 0);

        // Shift by more than 360 degrees - should wrap
        let shifted = red.adjust_hue(400.0);
        let (h, _, _) = shifted.to_hsl();

        // 400 % 360 = 40
        let expected_h = 40.0;
        assert!((h - expected_h).abs() < 2.0);
    }

    #[test]
    fn test_adjust_hue_negative() {
        let red = Color::Rgb(255, 0, 0);

        // Negative shift should work (wraps around)
        let shifted = red.adjust_hue(-60.0);
        let (h, _, _) = shifted.to_hsl();

        // Should be in valid range
        assert!((0.0..360.0).contains(&h));
    }

    #[test]
    fn test_from_hex_invalid_length() {
        assert!(Color::from_hex("#FF").is_none());
        assert!(Color::from_hex("#FFFF").is_none());
        assert!(Color::from_hex("#FF0000FF").is_none());
    }

    #[test]
    fn test_from_hex_invalid_characters() {
        assert!(Color::from_hex("#GGGGGG").is_none());
        assert!(Color::from_hex("#12345G").is_none());
        assert!(Color::from_hex("#ZZZZZZ").is_none());
    }

    #[test]
    fn test_from_hex_without_hash() {
        let color = Color::from_hex("FF0000").unwrap();
        assert_eq!(color.to_rgb(), (255, 0, 0));
    }

    #[test]
    fn test_from_hex_lowercase() {
        let color = Color::from_hex("#ff8040").unwrap();
        assert_eq!(color.to_rgb(), (255, 128, 64));
    }

    #[test]
    fn test_luminance_calculation() {
        // Black should have luminance ~0
        let black = Color::Rgb(0, 0, 0);
        assert!(black.luminance() < 0.01);

        // White should have luminance ~1
        let white = Color::Rgb(255, 255, 255);
        assert!(white.luminance() > 0.9);

        // Red should have lower luminance than green (green is more visible)
        let red = Color::Rgb(255, 0, 0);
        let green = Color::Rgb(0, 255, 0);
        assert!(green.luminance() > red.luminance());
    }

    #[test]
    fn test_adjust_brightness_normalized_edge_cases() {
        // Test brightening pure black
        let (r, g, b) = adjust_brightness_normalized(0.0, 0.0, 0.0, 0.5);
        assert!(r > 0.4 && r < 0.6);
        assert!(g > 0.4 && g < 0.6);
        assert!(b > 0.4 && b < 0.6);

        // Test dimming pure white
        let (r, g, b) = adjust_brightness_normalized(1.0, 1.0, 1.0, 0.5);
        assert!(r > 0.4 && r < 0.6);
        assert!(g > 0.4 && g < 0.6);
        assert!(b > 0.4 && b < 0.6);
    }

    #[test]
    fn test_hsl_roundtrip_edge_cases() {
        // Test grayscale colors
        let gray = Color::Rgb(128, 128, 128);
        let (h, s, l) = gray.to_hsl();
        let back = Color::from_hsl(h, s, l);
        let (r, g, b) = back.to_rgb();
        // Grayscale should have saturation ~0
        assert!(s < 1.0);
        assert!((r as i16 - g as i16).abs() < 2);
        assert!((g as i16 - b as i16).abs() < 2);
    }
}
