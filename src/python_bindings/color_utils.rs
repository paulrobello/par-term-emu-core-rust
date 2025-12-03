//! Python bindings for color utility functions
use crate::color::Color;
use pyo3::prelude::*;

/// Calculate perceived brightness of an RGB color using NTSC formula.
///
/// The NTSC formula weights color components based on human perception:
/// - Red: 30%
/// - Green: 59%
/// - Blue: 11%
///
/// Args:
///     r (int): Red component (0-255)
///     g (int): Green component (0-255)
///     b (int): Blue component (0-255)
///
/// Returns:
///     float: Perceived brightness value (0.0-1.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import perceived_brightness_rgb
///     >>> brightness = perceived_brightness_rgb(128, 128, 128)
///     >>> print(f"Gray brightness: {brightness:.2f}")
///     Gray brightness: 0.50
#[pyfunction]
#[pyo3(name = "perceived_brightness_rgb")]
pub fn py_perceived_brightness_rgb(r: u8, g: u8, b: u8) -> f64 {
    crate::color_utils::perceived_brightness_rgb(r, g, b)
}

/// Adjust foreground color to maintain minimum contrast against background.
///
/// Implements iTerm2's minimum contrast algorithm using NTSC perceived brightness.
/// The algorithm preserves the color's hue while adjusting brightness to ensure
/// readability.
///
/// Args:
///     fg (tuple): Foreground color (r, g, b) where each component is 0-255
///     bg (tuple): Background color (r, g, b) where each component is 0-255
///     minimum_contrast (float): Minimum required brightness difference (0.0-1.0)
///
/// Returns:
///     tuple: Adjusted foreground color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_contrast_rgb
///     >>> # Dark gray text on black background - will be lightened
///     >>> fg = (64, 64, 64)
///     >>> bg = (0, 0, 0)
///     >>> adjusted = adjust_contrast_rgb(fg, bg, 0.5)
///     >>> print(f"Adjusted color: {adjusted}")
///     Adjusted color: (128, 128, 128)
#[pyfunction]
#[pyo3(name = "adjust_contrast_rgb")]
pub fn py_adjust_contrast_rgb(
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    minimum_contrast: f64,
) -> (u8, u8, u8) {
    crate::color_utils::adjust_contrast_rgb(fg, bg, minimum_contrast)
}

/// Lighten an RGB color by a given amount.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///     amount (float): Amount to lighten (0.0 to 1.0)
///
/// Returns:
///     tuple: Lightened color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import lighten_rgb
///     >>> lightened = lighten_rgb((128, 64, 32), 0.5)
///     >>> print(f"Lightened: {lightened}")
#[pyfunction]
#[pyo3(name = "lighten_rgb")]
pub fn py_lighten_rgb(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.lighten(amount).to_rgb()
}

/// Darken an RGB color by a given amount.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///     amount (float): Amount to darken (0.0 to 1.0)
///
/// Returns:
///     tuple: Darkened color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import darken_rgb
///     >>> darkened = darken_rgb((200, 150, 100), 0.3)
///     >>> print(f"Darkened: {darkened}")
#[pyfunction]
#[pyo3(name = "darken_rgb")]
pub fn py_darken_rgb(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.darken(amount).to_rgb()
}

/// Calculate WCAG relative luminance of an RGB color.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     float: Relative luminance (0.0-1.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import color_luminance
///     >>> lum = color_luminance((255, 255, 255))
///     >>> print(f"White luminance: {lum:.2f}")
#[pyfunction]
#[pyo3(name = "color_luminance")]
pub fn py_color_luminance(rgb: (u8, u8, u8)) -> f32 {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.luminance()
}

/// Check if a color is dark (luminance < 0.5).
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     bool: True if color is dark
///
/// Example:
///     >>> from par_term_emu_core_rust import is_dark_color
///     >>> print(is_dark_color((50, 50, 50)))
///     True
#[pyfunction]
#[pyo3(name = "is_dark_color")]
pub fn py_is_dark_color(rgb: (u8, u8, u8)) -> bool {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.is_dark()
}

/// Calculate WCAG contrast ratio between two colors.
///
/// Args:
///     rgb1 (tuple): First color (r, g, b)
///     rgb2 (tuple): Second color (r, g, b)
///
/// Returns:
///     float: Contrast ratio (1.0 to 21.0)
///
/// Example:
///     >>> from par_term_emu_core_rust import contrast_ratio
///     >>> ratio = contrast_ratio((0, 0, 0), (255, 255, 255))
///     >>> print(f"Black/White ratio: {ratio:.1f}:1")
///     Black/White ratio: 21.0:1
#[pyfunction]
#[pyo3(name = "contrast_ratio")]
pub fn py_contrast_ratio(rgb1: (u8, u8, u8), rgb2: (u8, u8, u8)) -> f32 {
    let color1 = Color::Rgb(rgb1.0, rgb1.1, rgb1.2);
    let color2 = Color::Rgb(rgb2.0, rgb2.1, rgb2.2);
    color1.contrast_ratio(&color2)
}

/// Check if two colors meet WCAG AA standard (4.5:1 for normal text).
///
/// Args:
///     fg (tuple): Foreground color (r, g, b)
///     bg (tuple): Background color (r, g, b)
///
/// Returns:
///     bool: True if colors meet WCAG AA standard
///
/// Example:
///     >>> from par_term_emu_core_rust import meets_wcag_aa
///     >>> print(meets_wcag_aa((0, 0, 0), (255, 255, 255)))
///     True
#[pyfunction]
#[pyo3(name = "meets_wcag_aa")]
pub fn py_meets_wcag_aa(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> bool {
    let fg_color = Color::Rgb(fg.0, fg.1, fg.2);
    let bg_color = Color::Rgb(bg.0, bg.1, bg.2);
    fg_color.meets_wcag_aa(&bg_color)
}

/// Check if two colors meet WCAG AAA standard (7:1 for normal text).
///
/// Args:
///     fg (tuple): Foreground color (r, g, b)
///     bg (tuple): Background color (r, g, b)
///
/// Returns:
///     bool: True if colors meet WCAG AAA standard
///
/// Example:
///     >>> from par_term_emu_core_rust import meets_wcag_aaa
///     >>> print(meets_wcag_aaa((0, 0, 0), (255, 255, 255)))
///     True
#[pyfunction]
#[pyo3(name = "meets_wcag_aaa")]
pub fn py_meets_wcag_aaa(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> bool {
    let fg_color = Color::Rgb(fg.0, fg.1, fg.2);
    let bg_color = Color::Rgb(bg.0, bg.1, bg.2);
    fg_color.meets_wcag_aaa(&bg_color)
}

/// Mix two colors with a given ratio.
///
/// Args:
///     rgb1 (tuple): First color (r, g, b)
///     rgb2 (tuple): Second color (r, g, b)
///     ratio (float): Mix ratio (0.0 = all rgb1, 1.0 = all rgb2)
///
/// Returns:
///     tuple: Mixed color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import mix_colors
///     >>> mixed = mix_colors((255, 0, 0), (0, 0, 255), 0.5)
///     >>> print(f"Purple: {mixed}")
#[pyfunction]
#[pyo3(name = "mix_colors")]
pub fn py_mix_colors(rgb1: (u8, u8, u8), rgb2: (u8, u8, u8), ratio: f32) -> (u8, u8, u8) {
    let color1 = Color::Rgb(rgb1.0, rgb1.1, rgb1.2);
    let color2 = Color::Rgb(rgb2.0, rgb2.1, rgb2.2);
    color1.mix(&color2, ratio).to_rgb()
}

/// Convert RGB to HSL color space.
///
/// Args:
///     rgb (tuple): Color as (r, g, b) where each component is 0-255
///
/// Returns:
///     tuple: (hue, saturation, lightness) where:
///         - hue is in degrees (0-360)
///         - saturation is percentage (0-100)
///         - lightness is percentage (0-100)
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_hsl
///     >>> h, s, l = rgb_to_hsl((255, 0, 0))
///     >>> print(f"Red in HSL: H={h:.0f}° S={s:.0f}% L={l:.0f}%")
///     Red in HSL: H=0° S=100% L=50%
#[pyfunction]
#[pyo3(name = "rgb_to_hsl")]
pub fn py_rgb_to_hsl(rgb: (u8, u8, u8)) -> (f32, f32, f32) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_hsl()
}

/// Convert HSL to RGB color space.
///
/// Args:
///     h (float): Hue in degrees (0-360)
///     s (float): Saturation percentage (0-100)
///     l (float): Lightness percentage (0-100)
///
/// Returns:
///     tuple: RGB color (r, g, b) where each component is 0-255
///
/// Example:
///     >>> from par_term_emu_core_rust import hsl_to_rgb
///     >>> rgb = hsl_to_rgb(120, 100, 50)  # Pure green
///     >>> print(f"Green RGB: {rgb}")
///     Green RGB: (0, 255, 0)
#[pyfunction]
#[pyo3(name = "hsl_to_rgb")]
pub fn py_hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    Color::from_hsl(h, s, l).to_rgb()
}

/// Adjust color saturation.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///     amount (float): Amount to adjust saturation (-100 to 100)
///
/// Returns:
///     tuple: Adjusted color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_saturation
///     >>> saturated = adjust_saturation((200, 100, 100), 50)
///     >>> desaturated = adjust_saturation((200, 100, 100), -50)
#[pyfunction]
#[pyo3(name = "adjust_saturation")]
pub fn py_adjust_saturation(rgb: (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.adjust_saturation(amount).to_rgb()
}

/// Adjust color hue.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///     degrees (float): Degrees to rotate hue (wraps around 360)
///
/// Returns:
///     tuple: Adjusted color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import adjust_hue
///     >>> shifted = adjust_hue((255, 0, 0), 120)  # Red -> Green
#[pyfunction]
#[pyo3(name = "adjust_hue")]
pub fn py_adjust_hue(rgb: (u8, u8, u8), degrees: f32) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.adjust_hue(degrees).to_rgb()
}

/// Get complementary color (opposite on color wheel).
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     tuple: Complementary color (r, g, b)
///
/// Example:
///     >>> from par_term_emu_core_rust import complementary_color
///     >>> comp = complementary_color((255, 0, 0))  # Red -> Cyan
///     >>> print(f"Complement of red: {comp}")
#[pyfunction]
#[pyo3(name = "complementary_color")]
pub fn py_complementary_color(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.complementary().to_rgb()
}

/// Convert RGB to hex string.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     str: Hex color string (e.g., "#FF0000")
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_hex
///     >>> hex_str = rgb_to_hex((255, 128, 64))
///     >>> print(hex_str)
///     #FF8040
#[pyfunction]
#[pyo3(name = "rgb_to_hex")]
pub fn py_rgb_to_hex(rgb: (u8, u8, u8)) -> String {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_hex()
}

/// Convert hex string to RGB.
///
/// Args:
///     hex_str (str): Hex color string (e.g., "#FF0000" or "FF0000")
///
/// Returns:
///     tuple or None: RGB color (r, g, b) or None if invalid
///
/// Example:
///     >>> from par_term_emu_core_rust import hex_to_rgb
///     >>> rgb = hex_to_rgb("#FF8040")
///     >>> print(f"RGB: {rgb}")
///     RGB: (255, 128, 64)
#[pyfunction]
#[pyo3(name = "hex_to_rgb")]
pub fn py_hex_to_rgb(hex_str: &str) -> Option<(u8, u8, u8)> {
    Color::from_hex(hex_str).map(|c| c.to_rgb())
}

/// Convert RGB to nearest 256-color ANSI palette index.
///
/// Args:
///     rgb (tuple): Color as (r, g, b)
///
/// Returns:
///     int: ANSI 256-color index (16-255)
///
/// Example:
///     >>> from par_term_emu_core_rust import rgb_to_ansi_256
///     >>> idx = rgb_to_ansi_256((255, 0, 0))
///     >>> print(f"Red is closest to ANSI color {idx}")
#[pyfunction]
#[pyo3(name = "rgb_to_ansi_256")]
pub fn py_rgb_to_ansi_256(rgb: (u8, u8, u8)) -> u8 {
    let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
    color.to_ansi_256()
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Tests for perceived_brightness_rgb
    // =========================================================================

    #[test]
    fn test_perceived_brightness_black() {
        let brightness = py_perceived_brightness_rgb(0, 0, 0);
        assert!(brightness < 0.01, "Black should have near-zero brightness");
    }

    #[test]
    fn test_perceived_brightness_white() {
        let brightness = py_perceived_brightness_rgb(255, 255, 255);
        assert!(brightness > 0.99, "White should have near-1.0 brightness");
    }

    #[test]
    fn test_perceived_brightness_gray() {
        let brightness = py_perceived_brightness_rgb(128, 128, 128);
        assert!(
            brightness > 0.45 && brightness < 0.55,
            "Gray should have ~0.5 brightness, got {}",
            brightness
        );
    }

    #[test]
    fn test_perceived_brightness_green_brightest() {
        // Green has highest perceptual weight (59%), so pure green should appear brightest
        let red = py_perceived_brightness_rgb(255, 0, 0);
        let green = py_perceived_brightness_rgb(0, 255, 0);
        let blue = py_perceived_brightness_rgb(0, 0, 255);
        assert!(green > red, "Green should appear brighter than red");
        assert!(green > blue, "Green should appear brighter than blue");
    }

    // =========================================================================
    // Tests for adjust_contrast_rgb
    // =========================================================================

    #[test]
    fn test_adjust_contrast_dark_on_dark() {
        // Dark gray text on black should be lightened
        let adjusted = py_adjust_contrast_rgb((64, 64, 64), (0, 0, 0), 0.5);
        let original_brightness = py_perceived_brightness_rgb(64, 64, 64);
        let adjusted_brightness = py_perceived_brightness_rgb(adjusted.0, adjusted.1, adjusted.2);
        assert!(
            adjusted_brightness >= original_brightness,
            "Dark on dark should be lightened"
        );
    }

    #[test]
    fn test_adjust_contrast_light_on_light() {
        // Light gray text on white should be darkened
        let adjusted = py_adjust_contrast_rgb((220, 220, 220), (255, 255, 255), 0.5);
        let original_brightness = py_perceived_brightness_rgb(220, 220, 220);
        let adjusted_brightness = py_perceived_brightness_rgb(adjusted.0, adjusted.1, adjusted.2);
        assert!(
            adjusted_brightness <= original_brightness,
            "Light on light should be darkened"
        );
    }

    #[test]
    fn test_adjust_contrast_already_sufficient() {
        // Black on white already has maximum contrast
        let adjusted = py_adjust_contrast_rgb((0, 0, 0), (255, 255, 255), 0.5);
        assert_eq!(adjusted, (0, 0, 0), "Good contrast should not be changed");
    }

    #[test]
    fn test_adjust_contrast_zero_minimum() {
        // Zero minimum contrast should not change anything
        let adjusted = py_adjust_contrast_rgb((64, 64, 64), (0, 0, 0), 0.0);
        assert_eq!(adjusted, (64, 64, 64));
    }

    // =========================================================================
    // Tests for lighten_rgb
    // =========================================================================

    #[test]
    fn test_lighten_rgb() {
        let original = (100, 50, 25);
        let lightened = py_lighten_rgb(original, 0.5);
        assert!(
            lightened.0 >= original.0,
            "Red should increase when lightening"
        );
        assert!(
            lightened.1 >= original.1,
            "Green should increase when lightening"
        );
        assert!(
            lightened.2 >= original.2,
            "Blue should increase when lightening"
        );
    }

    #[test]
    fn test_lighten_rgb_zero() {
        let original = (100, 100, 100);
        let lightened = py_lighten_rgb(original, 0.0);
        assert_eq!(lightened, original, "Zero lighten should not change color");
    }

    #[test]
    fn test_lighten_rgb_max() {
        let lightened = py_lighten_rgb((0, 0, 0), 1.0);
        assert_eq!(
            lightened,
            (255, 255, 255),
            "Max lighten should produce white"
        );
    }

    // =========================================================================
    // Tests for darken_rgb
    // =========================================================================

    #[test]
    fn test_darken_rgb() {
        let original = (200, 150, 100);
        let darkened = py_darken_rgb(original, 0.5);
        assert!(
            darkened.0 <= original.0,
            "Red should decrease when darkening"
        );
        assert!(
            darkened.1 <= original.1,
            "Green should decrease when darkening"
        );
        assert!(
            darkened.2 <= original.2,
            "Blue should decrease when darkening"
        );
    }

    #[test]
    fn test_darken_rgb_zero() {
        let original = (100, 100, 100);
        let darkened = py_darken_rgb(original, 0.0);
        assert_eq!(darkened, original, "Zero darken should not change color");
    }

    #[test]
    fn test_darken_rgb_max() {
        let darkened = py_darken_rgb((255, 255, 255), 1.0);
        assert_eq!(darkened, (0, 0, 0), "Max darken should produce black");
    }

    // =========================================================================
    // Tests for color_luminance
    // =========================================================================

    #[test]
    fn test_color_luminance_black() {
        let lum = py_color_luminance((0, 0, 0));
        assert!(lum < 0.01, "Black should have near-zero luminance");
    }

    #[test]
    fn test_color_luminance_white() {
        let lum = py_color_luminance((255, 255, 255));
        assert!(lum > 0.99, "White should have near-1.0 luminance");
    }

    // =========================================================================
    // Tests for is_dark_color
    // =========================================================================

    #[test]
    fn test_is_dark_color() {
        assert!(py_is_dark_color((0, 0, 0)), "Black should be dark");
        assert!(py_is_dark_color((50, 50, 50)), "Dark gray should be dark");
        assert!(
            !py_is_dark_color((255, 255, 255)),
            "White should not be dark"
        );
        assert!(
            !py_is_dark_color((200, 200, 200)),
            "Light gray should not be dark"
        );
    }

    // =========================================================================
    // Tests for contrast_ratio
    // =========================================================================

    #[test]
    fn test_contrast_ratio_black_white() {
        let ratio = py_contrast_ratio((0, 0, 0), (255, 255, 255));
        assert!(
            ratio > 20.0 && ratio <= 21.0,
            "Black/white contrast should be ~21:1, got {}",
            ratio
        );
    }

    #[test]
    fn test_contrast_ratio_same_color() {
        let ratio = py_contrast_ratio((128, 128, 128), (128, 128, 128));
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "Same color contrast should be 1:1"
        );
    }

    #[test]
    fn test_contrast_ratio_symmetric() {
        let ratio1 = py_contrast_ratio((100, 50, 25), (200, 150, 100));
        let ratio2 = py_contrast_ratio((200, 150, 100), (100, 50, 25));
        assert!(
            (ratio1 - ratio2).abs() < 0.01,
            "Contrast ratio should be symmetric"
        );
    }

    // =========================================================================
    // Tests for meets_wcag_aa
    // =========================================================================

    #[test]
    fn test_meets_wcag_aa() {
        // Black on white should easily pass
        assert!(py_meets_wcag_aa((0, 0, 0), (255, 255, 255)));
        // Similar grays should fail
        assert!(!py_meets_wcag_aa((128, 128, 128), (140, 140, 140)));
    }

    // =========================================================================
    // Tests for meets_wcag_aaa
    // =========================================================================

    #[test]
    fn test_meets_wcag_aaa() {
        // Black on white should easily pass
        assert!(py_meets_wcag_aaa((0, 0, 0), (255, 255, 255)));
        // Similar grays should fail
        assert!(!py_meets_wcag_aaa((128, 128, 128), (140, 140, 140)));
    }

    // =========================================================================
    // Tests for mix_colors
    // =========================================================================

    #[test]
    fn test_mix_colors_equal() {
        let mixed = py_mix_colors((255, 0, 0), (0, 0, 255), 0.5);
        // Equal mix of red and blue should be purple-ish
        assert!(mixed.0 > 100, "Should have red component");
        assert!(mixed.2 > 100, "Should have blue component");
    }

    #[test]
    fn test_mix_colors_zero_ratio() {
        let mixed = py_mix_colors((255, 0, 0), (0, 0, 255), 0.0);
        assert_eq!(mixed, (255, 0, 0), "Ratio 0 should be all first color");
    }

    #[test]
    fn test_mix_colors_one_ratio() {
        let mixed = py_mix_colors((255, 0, 0), (0, 0, 255), 1.0);
        assert_eq!(mixed, (0, 0, 255), "Ratio 1 should be all second color");
    }

    // =========================================================================
    // Tests for rgb_to_hsl and hsl_to_rgb
    // =========================================================================

    #[test]
    fn test_rgb_to_hsl_red() {
        let (h, s, l) = py_rgb_to_hsl((255, 0, 0));
        assert!(!(1.0..=359.0).contains(&h), "Red hue should be ~0°");
        assert!((s - 100.0).abs() < 1.0, "Red should be fully saturated");
        assert!((l - 50.0).abs() < 1.0, "Red should have 50% lightness");
    }

    #[test]
    fn test_rgb_to_hsl_green() {
        let (h, _s, _l) = py_rgb_to_hsl((0, 255, 0));
        assert!(
            (h - 120.0).abs() < 1.0,
            "Green hue should be ~120°, got {}",
            h
        );
    }

    #[test]
    fn test_rgb_to_hsl_blue() {
        let (h, _s, _l) = py_rgb_to_hsl((0, 0, 255));
        assert!(
            (h - 240.0).abs() < 1.0,
            "Blue hue should be ~240°, got {}",
            h
        );
    }

    #[test]
    fn test_hsl_to_rgb_roundtrip() {
        let original = (200, 100, 50);
        let (h, s, l) = py_rgb_to_hsl(original);
        let converted = py_hsl_to_rgb(h, s, l);
        assert!(
            (original.0 as i16 - converted.0 as i16).abs() <= 1,
            "Red channel mismatch"
        );
        assert!(
            (original.1 as i16 - converted.1 as i16).abs() <= 1,
            "Green channel mismatch"
        );
        assert!(
            (original.2 as i16 - converted.2 as i16).abs() <= 1,
            "Blue channel mismatch"
        );
    }

    // =========================================================================
    // Tests for adjust_saturation
    // =========================================================================

    #[test]
    fn test_adjust_saturation_increase() {
        let original = (200, 100, 100);
        let saturated = py_adjust_saturation(original, 50.0);
        // Saturating a reddish color should increase red vs other channels
        assert!(
            saturated.0 >= original.0 || saturated.1 <= original.1,
            "Saturation increase should make color more vivid"
        );
    }

    #[test]
    fn test_adjust_saturation_decrease() {
        let original = (200, 100, 100);
        let desaturated = py_adjust_saturation(original, -100.0);
        // Full desaturation should produce a gray
        assert!(
            (desaturated.0 as i16 - desaturated.1 as i16).abs() <= 5,
            "Full desaturation should produce gray"
        );
    }

    // =========================================================================
    // Tests for adjust_hue
    // =========================================================================

    #[test]
    fn test_adjust_hue_180_degrees() {
        // Shifting hue by 180° should give complementary color
        let shifted = py_adjust_hue((255, 0, 0), 180.0);
        // Red shifted by 180° should be cyan-ish
        assert!(shifted.1 > 200, "Should have high green");
        assert!(shifted.2 > 200, "Should have high blue");
    }

    // =========================================================================
    // Tests for complementary_color
    // =========================================================================

    #[test]
    fn test_complementary_red() {
        let comp = py_complementary_color((255, 0, 0));
        // Complementary of red should be cyan
        assert!(
            comp.1 > 200 || comp.2 > 200,
            "Complement of red should be cyan-ish"
        );
    }

    // =========================================================================
    // Tests for rgb_to_hex and hex_to_rgb
    // =========================================================================

    #[test]
    fn test_rgb_to_hex() {
        assert_eq!(py_rgb_to_hex((255, 0, 0)), "#FF0000");
        assert_eq!(py_rgb_to_hex((0, 255, 0)), "#00FF00");
        assert_eq!(py_rgb_to_hex((0, 0, 255)), "#0000FF");
        assert_eq!(py_rgb_to_hex((128, 128, 128)), "#808080");
    }

    #[test]
    fn test_hex_to_rgb_with_hash() {
        assert_eq!(py_hex_to_rgb("#FF0000"), Some((255, 0, 0)));
        assert_eq!(py_hex_to_rgb("#00FF00"), Some((0, 255, 0)));
        assert_eq!(py_hex_to_rgb("#0000FF"), Some((0, 0, 255)));
    }

    #[test]
    fn test_hex_to_rgb_without_hash() {
        assert_eq!(py_hex_to_rgb("FF0000"), Some((255, 0, 0)));
        assert_eq!(py_hex_to_rgb("808080"), Some((128, 128, 128)));
    }

    #[test]
    fn test_hex_to_rgb_invalid() {
        assert_eq!(py_hex_to_rgb("invalid"), None);
        assert_eq!(py_hex_to_rgb("GGGGGG"), None);
        assert_eq!(py_hex_to_rgb("12345"), None); // Wrong length
    }

    #[test]
    fn test_hex_roundtrip() {
        let original = (200, 100, 50);
        let hex = py_rgb_to_hex(original);
        let converted = py_hex_to_rgb(&hex).unwrap();
        assert_eq!(original, converted);
    }

    // =========================================================================
    // Tests for rgb_to_ansi_256
    // =========================================================================

    #[test]
    fn test_rgb_to_ansi_256_colors() {
        // Test some standard colors
        let black = py_rgb_to_ansi_256((0, 0, 0));
        let white = py_rgb_to_ansi_256((255, 255, 255));
        assert!(
            !(17..=230).contains(&black),
            "Black should map to start or grayscale"
        );
        assert!(
            white > 230 || white == 15,
            "White should map to end of grayscale or bright white"
        );
    }
}
