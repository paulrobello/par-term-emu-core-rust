//! Color manipulation and conversion utilities

use crate::color::Color;

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
}
